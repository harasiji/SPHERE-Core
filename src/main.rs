use eframe::{egui, App, Frame};
use egui::{Color32, FontId, RichText};
use serialport::{SerialPortType, UsbPortInfo};
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const ESP_VIDS: &[u16] = &[0x303A, 0x10C4, 0x1A86];

struct EspFinderApp {
    log: Vec<String>,
    connected_ports: HashSet<String>,
    last_scan: Instant,
    firmware_path: String,
    flash_address: String,
    is_flashing: bool,
    flash_progress: f32,
    flash_rx: Option<Receiver<String>>,
    file_rx: Option<Receiver<Option<String>>>,
}

impl Default for EspFinderApp {
    fn default() -> Self {
        Self {
            log: vec![">> SPHERE SYSTEM INITIALIZED.".to_string()],
            connected_ports: HashSet::new(),
            last_scan: Instant::now() - Duration::from_secs(10),
            firmware_path: String::new(),
            flash_address: "0x0".to_string(),
            is_flashing: false,
            flash_progress: 0.0,
            flash_rx: None,
            file_rx: None,
        }
    }
}

impl EspFinderApp {
    fn scan_ports(&mut self) {
        let mut current_ports = HashSet::new();
        let mut new_logs = Vec::new();

        if let Ok(ports) = serialport::available_ports() {
            for port in ports {
                if let SerialPortType::UsbPort(UsbPortInfo { vid, pid, .. }) = &port.port_type {
                    if ESP_VIDS.contains(vid) {
                        current_ports.insert(port.port_name.clone());
                        if !self.connected_ports.contains(&port.port_name) {
                            let chip = Self::detect_chip(&port.port_name);
                            new_logs.push(format!("[+] CONNECTED: {}", port.port_name));
                            new_logs.push(format!("    VID:PID -> {:04x}:{:04x}", vid, pid));
                            new_logs.push(format!("    CHIP    -> {}", chip.to_uppercase()));
                        }
                    }
                }
            }
        }

        for old in &self.connected_ports {
            if !current_ports.contains(old) {
                new_logs.push(format!("[-] DISCONNECTED: {}", old));
            }
        }

        self.connected_ports = current_ports;
        self.log.extend(new_logs);
    }

    fn detect_chip(port_name: &str) -> String {
        let output = Command::new("espflash")
            .args(["board-info", "--port", port_name, "--no-stub"])
            .output();
        match output {
            Ok(out) => {
                let combined = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                if !out.status.success() {
                    return "PROBE FAILED".into();
                }
                for line in combined.lines() {
                    let lower = line.to_lowercase();
                    if lower.contains("chip type:") || lower.contains("chip is") {
                        return line.split_whitespace().collect::<Vec<_>>().join(" ");
                    }
                }
                "UNKNOWN CHIP".into()
            }
            Err(_) => "EXEC ERROR".into(),
        }
    }

    fn hard_reset_esp(port_name: &str) -> Result<(), String> {
        let mut port = serialport::new(port_name, 115200)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("Ошибка открытия порта для сброса: {}", e))?;

        port.write_data_terminal_ready(false)
            .map_err(|e| e.to_string())?;
        port.write_request_to_send(true)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(100));

        port.write_request_to_send(false)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(50));

        port.write_data_terminal_ready(true)
            .map_err(|e| e.to_string())?;
        drop(port);
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    fn reboot_device(port_name: &str) -> Result<(), String> {
        let mut port = serialport::new(port_name, 115200)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("Ошибка открытия порта для перезагрузки: {}", e))?;

        port.write_data_terminal_ready(true)
            .map_err(|e| e.to_string())?;
        port.write_request_to_send(true)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(100));

        port.write_request_to_send(false)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(50));

        port.write_data_terminal_ready(false)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(300));

        drop(port);
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    fn flash_firmware(&mut self, port_name: String) {
        if self.firmware_path.is_empty() {
            self.log
                .push("!! ERROR: FIRMWARE PATH IS EMPTY".to_string());
            return;
        }

        self.log.push(format!(">> FLASHING: {}", port_name));
        self.is_flashing = true;
        self.flash_progress = 0.0;
        let (tx, rx) = mpsc::channel();
        self.flash_rx = Some(rx);

        let path = self.firmware_path.clone();
        let addr = self
            .flash_address
            .trim()
            .trim_start_matches("0x")
            .to_string();
        let port_name_clone = port_name.clone();
        let tx_reset = tx.clone();

        std::thread::spawn(move || {
            let _ = tx_reset.send(">> RESETTING DEVICE FOR FLASHING...".to_string());
            if let Err(e) = Self::hard_reset_esp(&port_name_clone) {
                let _ = tx_reset.send(format!("!! RESET WARNING: {}", e));
            }

            let mut child = Command::new("espflash")
                .args([
                    "write-bin",
                    "--no-stub",
                    "--port",
                    &port_name_clone,
                    &addr,
                    &path,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();

            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();

            let tx_err = tx.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let _ = tx_err.send(format!("ERR: {}", line));
                }
            });

            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                let _ = tx.send(line);
            }

            let status = child.wait().unwrap();
            if status.success() {
                let _ = tx.send(">> FLASH SUCCESSFUL!".to_string());
            } else {
                let _ = tx.send("!! FLASH FAILED".to_string());
            }
        });
    }

    fn draw_progress_bar(&self, ui: &mut egui::Ui) {
        if !self.is_flashing && self.flash_progress == 0.0 {
            return;
        }

        let width = 40;
        let filled = (self.flash_progress * width as f32).round() as usize;
        let empty = width - filled;

        let bar = format!(
            "[{}{}] {:>3.0}%",
            "=".repeat(filled),
            " ".repeat(empty),
            self.flash_progress * 100.0
        );

        let color = if self.flash_progress >= 1.0 {
            Color32::from_rgb(0, 255, 150)
        } else {
            Color32::from_rgb(0, 200, 255)
        };

        ui.label(
            RichText::new(bar)
                .color(color)
                .font(FontId::monospace(14.0)),
        );
    }
}

impl App for EspFinderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        if let Some(rx) = &self.file_rx {
            if let Ok(Some(path)) = rx.try_recv() {
                self.firmware_path = path;
                self.file_rx = None;
            }
        }

        if let Some(rx) = &self.flash_rx {
            let mut finished = false;
            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        if let Some(start) = msg.find('(') {
                            if let Some(end) = msg[start..].find('%') {
                                let pct_str = &msg[start + 1..start + end].trim();
                                if let Ok(pct) = pct_str.parse::<f32>() {
                                    self.flash_progress = pct / 100.0;
                                }
                            }
                        }

                        if msg.starts_with(">> FLASH") || msg.starts_with("!!") {
                            finished = true;
                            if msg.contains("SUCCESSFUL") {
                                self.flash_progress = 1.0;
                            }
                        }
                        self.log.push(msg);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
            if finished {
                self.is_flashing = false;
                self.flash_rx = None;
            }
        }

        if self.last_scan.elapsed() >= Duration::from_secs(2) && !self.is_flashing {
            self.scan_ports();
            self.last_scan = Instant::now();
        }

        ctx.request_repaint_after(Duration::from_millis(100));

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.heading(
                    RichText::new("SPHERE // ESP WATCHER").color(Color32::from_rgb(0, 200, 255)),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let status = if self.connected_ports.is_empty() {
                        RichText::new("STATUS: WAITING").color(Color32::from_rgb(255, 100, 100))
                    } else {
                        RichText::new("STATUS: CONNECTED").color(Color32::from_rgb(0, 255, 150))
                    };
                    ui.label(status);
                });
            });
            ui.add_space(10.0);
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("File:");
                ui.add_enabled(
                    !self.is_flashing,
                    egui::TextEdit::singleline(&mut self.firmware_path).desired_width(250.0),
                );

                if ui
                    .add_enabled(!self.is_flashing, egui::Button::new("Browse"))
                    .clicked()
                {
                    let (tx, rx) = mpsc::channel();
                    self.file_rx = Some(rx);
                    std::thread::spawn(move || {
                        let file = rfd::FileDialog::new()
                            .add_filter("Binary", &["bin"])
                            .pick_file();
                        let _ = tx.send(file.map(|p| p.display().to_string()));
                    });
                }

                ui.separator();

                ui.label("Addr:");
                ui.add_enabled(
                    !self.is_flashing,
                    egui::TextEdit::singleline(&mut self.flash_address).desired_width(60.0),
                );

                let can_flash = !self.connected_ports.is_empty()
                    && !self.is_flashing
                    && !self.firmware_path.is_empty();

                if ui
                    .add_enabled(can_flash, egui::Button::new("FLASH"))
                    .clicked()
                {
                    if let Some(port) = self.connected_ports.iter().next().cloned() {
                        self.flash_firmware(port);
                    }
                }

                let can_reboot = !self.connected_ports.is_empty() && !self.is_flashing;
                if ui
                    .add_enabled(can_reboot, egui::Button::new("REBOOT"))
                    .clicked()
                {
                    if let Some(port) = self.connected_ports.iter().next().cloned() {
                        self.log.push(format!(">> REBOOTING DEVICE: {}", port));

                        std::thread::spawn(move || {
                            if let Err(e) = Self::reboot_device(&port) {
                                eprintln!("Reboot error: {}", e);
                            }
                        });

                        self.log.push(">> REBOOT SEQUENCE INITIATED.".to_string());
                    }
                }
            });
            ui.add_space(10.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.log {
                        let color = if line.contains("[+]") {
                            Color32::from_rgb(0, 255, 150)
                        } else if line.contains("[-]") || line.contains("!!") {
                            Color32::from_rgb(255, 100, 100)
                        } else if line.starts_with(">>") {
                            Color32::from_rgb(0, 200, 255)
                        } else {
                            Color32::from_rgb(200, 200, 200)
                        };

                        ui.label(
                            RichText::new(line)
                                .color(color)
                                .font(FontId::monospace(14.0)),
                        );
                    }

                    if self.is_flashing || self.flash_progress > 0.0 {
                        ui.add_space(10.0);
                        ui.separator();
                        self.draw_progress_bar(ui);
                    }
                });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([750.0, 550.0])
            .with_title("SPHERE // ESP Watcher"),
        ..Default::default()
    };

    eframe::run_native(
        "SPHERE",
        options,
        Box::new(|_cc| Box::new(EspFinderApp::default())),
    )
}
