// Release builds on Windows are GUI-only: no console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use bpm_detector::decode::decode_file;
use bpm_detector::tempo::{Confidence, Stability, TempoEstimate, estimate_tempo};
use eframe::egui;

const APP_NAME: &str = "BPM Detector";
const DROP_ZONE_HEIGHT: f32 = 150.0;
const BUTTON_HEIGHT: f32 = 44.0;
const HIGH_CONFIDENCE_COLOR: egui::Color32 = egui::Color32::from_rgb(64, 170, 96);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([420.0, 460.0])
            .with_min_inner_size([360.0, 440.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_| Ok(Box::new(BpmDetectorApp::default()))),
    )
}

enum Analysis {
    Idle,
    Running(Receiver<Result<TempoEstimate, String>>),
    Done(TempoEstimate),
    Failed(String),
}

struct BpmDetectorApp {
    file: Option<PathBuf>,
    analysis: Analysis,
}

impl Default for BpmDetectorApp {
    fn default() -> Self {
        Self {
            file: None,
            analysis: Analysis::Idle,
        }
    }
}

impl eframe::App for BpmDetectorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.collect_finished_analysis();

        let context = ui.ctx().clone();
        let (hovering_file, dropped_file) = context.input(|input| {
            (
                !input.raw.hovered_files.is_empty(),
                input
                    .raw
                    .dropped_files
                    .first()
                    .map(|file| file.path().to_path_buf()),
            )
        });
        if let Some(path) = dropped_file {
            self.file = Some(path);
            self.analysis = Analysis::Idle;
        }

        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 14.0;
            self.drop_zone(ui, hovering_file);

            let running = matches!(self.analysis, Analysis::Running(_));
            let button = egui::Button::new(egui::RichText::new("Detect BPM").size(18.0).strong())
                .min_size(egui::vec2(0.0, BUTTON_HEIGHT));
            let clicked = ui
                .vertical_centered_justified(|ui| {
                    ui.add_enabled(self.file.is_some() && !running, button)
                        .clicked()
                })
                .inner;
            if clicked {
                self.start_analysis(&context);
            }

            self.result(ui);
        });
    }
}

impl BpmDetectorApp {
    fn start_analysis(&mut self, context: &egui::Context) {
        let Some(path) = self.file.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let repaint_context = context.clone();
        thread::spawn(move || {
            // The receiver is gone when another file was dropped meanwhile: nothing to report.
            let _ = sender.send(analyze(&path));
            repaint_context.request_repaint();
        });
        self.analysis = Analysis::Running(receiver);
    }

    fn collect_finished_analysis(&mut self) {
        let Analysis::Running(receiver) = &self.analysis else {
            return;
        };
        self.analysis = match receiver.try_recv() {
            Ok(Ok(estimate)) => Analysis::Done(estimate),
            Ok(Err(message)) => Analysis::Failed(message),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Analysis::Failed("The analysis stopped unexpectedly".to_string())
            }
        };
    }

    fn drop_zone(&self, ui: &mut egui::Ui, hovering_file: bool) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), DROP_ZONE_HEIGHT),
            egui::Sense::hover(),
        );
        let visuals = ui.visuals();
        let accent = visuals.selection.bg_fill;
        let (fill, stroke) = if hovering_file {
            (accent.gamma_multiply(0.18), egui::Stroke::new(2.0, accent))
        } else {
            (
                visuals.extreme_bg_color,
                egui::Stroke::new(1.5, visuals.widgets.noninteractive.bg_stroke.color),
            )
        };
        let painter = ui.painter();
        painter.rect(rect, 12.0, fill, stroke, egui::StrokeKind::Inside);

        let (title, subtitle) = match &self.file {
            Some(path) => (file_name(path), "Drop another file to replace it"),
            None => (
                "Drop an audio file here".to_string(),
                "WAV · AIFF · MP3 · FLAC · M4A · OGG",
            ),
        };
        let text_width = rect.width() - 32.0;
        let title_galley =
            single_line_galley(ui, title, 17.0, visuals.strong_text_color(), text_width);
        let subtitle_galley = single_line_galley(
            ui,
            subtitle.to_string(),
            13.0,
            visuals.weak_text_color(),
            text_width,
        );
        let title_position =
            rect.center() - egui::vec2(title_galley.size().x / 2.0, title_galley.size().y + 2.0);
        let subtitle_position = rect.center() + egui::vec2(-subtitle_galley.size().x / 2.0, 4.0);
        painter.galley(title_position, title_galley, visuals.strong_text_color());
        painter.galley(
            subtitle_position,
            subtitle_galley,
            visuals.weak_text_color(),
        );
    }

    fn result(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| match &self.analysis {
            Analysis::Idle => {}
            Analysis::Running(_) => {
                ui.add_space(24.0);
                ui.spinner();
                ui.weak("Analyzing…");
            }
            Analysis::Failed(message) => {
                ui.add_space(24.0);
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
            Analysis::Done(estimate) => tempo_view(ui, estimate),
        });
    }
}

fn tempo_view(ui: &mut egui::Ui, estimate: &TempoEstimate) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.label(
        egui::RichText::new(format!("{:.2}", estimate.bpm))
            .size(64.0)
            .strong(),
    );
    ui.weak(egui::RichText::new("BPM").size(15.0));
    ui.add_space(10.0);

    let (confidence_text, confidence_color) = match estimate.confidence {
        Confidence::High => ("High", HIGH_CONFIDENCE_COLOR),
        Confidence::Medium => ("Medium", ui.visuals().warn_fg_color),
        Confidence::Low => ("Low", ui.visuals().error_fg_color),
    };
    ui.horizontal(|ui| {
        let label = format!("Confidence: {confidence_text}");
        let width = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    label.clone(),
                    egui::FontId::proportional(14.0),
                    confidence_color,
                )
                .size()
                .x
        });
        ui.add_space((ui.available_width() - width) / 2.0);
        ui.colored_label(confidence_color, egui::RichText::new(label).size(14.0));
    });

    ui.label(format!(
        "Half {:.2}  ·  Double {:.2}",
        estimate.bpm / 2.0,
        estimate.bpm * 2.0
    ));

    match estimate.stability {
        Stability::Steady => {
            ui.weak("Steady tempo");
        }
        Stability::Variable {
            first_half_bpm,
            second_half_bpm,
        } => {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("The tempo changes: {first_half_bpm:.2} BPM in the first half,\n{second_half_bpm:.2} BPM in the second."),
            );
        }
        Stability::Unknown => {
            ui.weak("Tempo stability not measurable on this file");
        }
    }
}

fn single_line_galley(
    ui: &egui::Ui,
    text: String,
    size: f32,
    color: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple(text, egui::FontId::proportional(size), color, max_width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn analyze(path: &Path) -> Result<TempoEstimate, String> {
    let audio = decode_file(path)?;
    estimate_tempo(&audio.samples, audio.sample_rate).map_err(|error| error.to_string())
}
