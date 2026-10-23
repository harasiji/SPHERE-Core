use eframe::{egui, App, Frame};
use egui::{Color32, FontFamily, FontId, Margin, Rounding, Stroke};
use serialport::{SerialPortType, UsbPortInfo};
use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant};

const ESP_VIDS: &[u16] = &[0x303A, 0x10C4, 0x1A86];

struct EspFinderApp {
    log: Vec<String>,
    connected_ports: HashSet<String>,
    last_scan: Instant,
    scan_interval: Duration,
}

impl Default for EspFinderApp {
    fn default() -> Self {
        let log = vec![
            "  ███████╗██████╗ ██╗  ██╗███████╗██████╗ ███████╗".to_string(),
            "  ██╔════╝██╔══██╗██║  ██║██╔════╝██╔══██╗██╔════╝".to_string(),
            "  ███████╗██████╔╝███████║█████╗  ██████╔╝█████╗  ".to_string(),
            "  ╚════██║██╔═══╝ ██║  ██║██╔══╝  ██╔══██╗██╔══╝  ".to_string(),
            "  ███████║██║     ██║  ██║███████╗██║  ██║███████╗".to_string(),
            "  ╚══════╝╚═╝     ╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚══════╝".to_string(),
            "".to_string(),
            "WATCHING PORTS...".to_string(),
        ];

        Self {
            log,
            connected_ports: HashSet::new(),
            last_scan: Instant::now() - Duration::from_secs(10),
            scan_interval: Duration::from_secs(2),
        }
    }
}

impl EspFinderApp {
    fn setup_custom_style(ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();

        style.text_styles = [
            (
                egui::TextStyle::Heading,
                FontId::new(22.0, FontFamily::Monospace),
            ),
            (
                egui::TextStyle::Body,
                FontId::new(15.0, FontFamily::Monospace),
            ),
            (
                egui::TextStyle::Button,
                FontId::new(16.0, FontFamily::Monospace),
            ),
            (
                egui::TextStyle::Monospace,
                FontId::new(14.0, FontFamily::Monospace),
            ),
            (
                egui::TextStyle::Small,
                FontId::new(10.0, FontFamily::Monospace),
            ),
        ]
        .into();

        let bg_color = Color32::from_rgb(15, 12, 18);
        let neon_pink = Color32::from_rgb(255, 0, 150);
        let neon_cyan = Color32::from_rgb(0, 255, 200);
        let text_color = Color32::from_rgb(200, 200, 210);

        style.visuals.dark_mode = true;
        style.visuals.override_text_color = Some(text_color);

        style.visuals.window_fill = bg_color;
        style.visuals.panel_fill = bg_color;

        style.visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.5_f32, neon_pink);
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, neon_pink);
        style.visuals.widgets.inactive.rounding = Rounding::same(4.0);

        style.visuals.widgets.hovered.bg_fill = neon_pink.linear_multiply(0.2);
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(2.0_f32, neon_pink);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
        style.visuals.widgets.hovered.rounding = Rounding::same(4.0);

        style.visuals.widgets.active.bg_fill = neon_cyan.linear_multiply(0.3);
        style.visuals.widgets.active.bg_stroke = Stroke::new(2.0_f32, neon_cyan);
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);

        style.visuals.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(40, 35, 50));

        ctx.set_style(style);
    }

    fn detect_chip(port_name: &str) -> String {
        let output = Command::new("espflash")
            .args(["board-info", "--port", port_name, "--no-stub"])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = format!("{}{}", stdout, stderr);

                if !out.status.success() {
                    return "PROBE FAILED".to_string();
                }

                for line in combined.lines() {
                    let lower = line.to_lowercase();
                    if lower.contains("chip type:") || lower.contains("chip is") {
                        return line.split_whitespace().collect::<Vec<_>>().join(" ");
                    }
                }
                "UNKNOWN CHIP".to_string()
            }
            Err(_) => "EXEC ERROR".to_string(),
        }
    }

    fn scan_ports(&mut self) {
        let mut current_ports = HashSet::new();
        let mut devices = Vec::new();

        if let Ok(ports) = serialport::available_ports() {
            for port in ports {
                if let SerialPortType::UsbPort(UsbPortInfo { vid, pid, .. }) = &port.port_type {
                    if ESP_VIDS.contains(vid) {
                        current_ports.insert(port.port_name.clone());
                        devices.push((port.port_name.clone(), *vid, *pid));
                    }
                }
            }
        }

        for (port_name, vid, pid) in devices {
            if !self.connected_ports.contains(&port_name) {
                let chip = Self::detect_chip(&port_name);
                self.log.push(format!("\n[+] CONNECTED: {}", port_name));
                self.log
                    .push(format!("    VID:PID -> {:04x}:{:04x}", vid, pid));
                self.log
                    .push(format!("    CHIP    -> {}", chip.to_uppercase()));
            }
        }

        for old_port in &self.connected_ports {
            if !current_ports.contains(old_port) {
                self.log.push(format!(
                    "\n[-] DISCONNECTED: {} \n \n   Please connect chip",
                    old_port
                ));
            }
        }

        self.connected_ports = current_ports;

        if self.log.len() > 100 {
            let excess = self.log.len() - 100;
            self.log.drain(0..excess);
        }
    }
}

impl App for EspFinderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        Self::setup_custom_style(ctx);

        if self.last_scan.elapsed() >= self.scan_interval {
            self.scan_ports();
            self.last_scan = Instant::now();
        }

        ctx.request_repaint_after(self.scan_interval);

        egui::TopBottomPanel::top("top_panel")
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(10, 8, 12))
                    .inner_margin(Margin::symmetric(20.0, 15.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("SPHERE")
                            .heading()
                            .color(Color32::from_rgb(0, 255, 200)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let status = if self.connected_ports.is_empty() {
                            "STATUS: WAITING\n Please connect chip"
                        } else {
                            "STATUS: CONNECTED"
                        };
                        let color = if self.connected_ports.is_empty() {
                            Color32::from_rgb(255, 0, 150)
                        } else {
                            Color32::from_rgb(0, 255, 200)
                        };
                        ui.label(egui::RichText::new(status).color(color).strong());
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(15, 12, 18))
                    .inner_margin(Margin::same(20.0)),
            )
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left(), rect.top()),
                        egui::pos2(rect.right(), rect.top()),
                    ],
                    Stroke::new(2.0_f32, Color32::from_rgb(255, 0, 150)),
                );

                ui.add_space(10.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log {
                            let color = if line.contains("[+]") || line.contains("CONNECTED") {
                                Color32::from_rgb(0, 255, 200)
                            } else if line.contains("[-]") || line.contains("DISCONNECTED") {
                                Color32::from_rgb(255, 100, 100)
                            } else if line.starts_with(">>") {
                                Color32::from_rgb(255, 0, 150)
                            } else if line.contains("████")
                                || line.contains("█╔")
                                || line.contains("╚")
                            {
                                Color32::from_rgb(255, 0, 150)
                            } else {
                                Color32::from_rgb(180, 180, 190)
                            };

                            ui.label(egui::RichText::new(line).color(color));
                        }
                    });
            });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([650.0, 550.0])
            .with_title("SPHERE"),
        ..Default::default()
    };

    eframe::run_native(
        "SPHERE",
        options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.window_fill = Color32::TRANSPARENT;
            cc.egui_ctx.set_visuals(visuals);

            Box::new(EspFinderApp::default())
        }),
    )
}
