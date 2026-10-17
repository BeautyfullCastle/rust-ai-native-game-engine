use egui::{Context, Ui};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};

enum Work {
    Create { output: PathBuf, seed: String },
    Open { root: PathBuf },
}

pub struct Panel {
    pub open: bool,
    pub root: String,
    pub output: String,
    pub seed: String,
    pub status: String,
    pending: Option<Receiver<Result<PathBuf, String>>>,
    ready: Option<PathBuf>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            root: String::new(),
            output: String::new(),
            seed: "room-starter".into(),
            status: String::new(),
            pending: None,
            ready: None,
        }
    }
}

impl Panel {
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.ready.is_some()
    }

    pub fn take_ready(&mut self) -> Option<PathBuf> {
        self.ready.take()
    }

    pub fn poll(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Room project worker disconnected before returning a result".into())
            }
        };
        self.pending = None;
        match result {
            Ok(root) => {
                self.root = root.display().to_string();
                self.status = format!("Room project validated: {}", root.display());
                self.ready = Some(root);
            }
            Err(error) => self.status = format!("Room project operation failed: {error}"),
        }
    }

    pub fn show(
        &mut self,
        context: &Context,
        blocked_reason: Option<&str>,
        mut authorize: impl FnMut() -> Result<(), String>,
    ) {
        self.poll();
        if !self.open {
            return;
        }
        let mut window_open = self.open;
        egui::Window::new("Room project")
            .open(&mut window_open)
            .default_width(520.0)
            .default_pos(egui::pos2(250.0, 80.0))
            .show(context, |ui| {
                ui.label(
                    "Create a new Windows Room project or open an existing validated project.",
                );
                if let Some(reason) = blocked_reason {
                    ui.label(reason);
                }
                ui.add_enabled_ui(!self.busy(), |ui| {
                    field(ui, "New Room project directory", &mut self.output);
                    field(ui, "Room project seed", &mut self.seed);
                    field(ui, "Existing Room project directory", &mut self.root);
                });
                ui.horizontal(|ui| {
                    let can_start = !self.busy() && blocked_reason.is_none();
                    if ui
                        .add_enabled(can_start, egui::Button::new("Create Room project"))
                        .clicked()
                    {
                        match authorize() {
                            Ok(()) => {
                                self.start(Work::Create {
                                    output: PathBuf::from(&self.output),
                                    seed: self.seed.clone(),
                                });
                            }
                            Err(error) => {
                                self.status = format!("Room project operation blocked: {error}")
                            }
                        }
                    }
                    if ui
                        .add_enabled(can_start, egui::Button::new("Open Room project"))
                        .clicked()
                    {
                        match authorize() {
                            Ok(()) => {
                                self.start(Work::Open {
                                    root: PathBuf::from(&self.root),
                                });
                            }
                            Err(error) => {
                                self.status = format!("Room project operation blocked: {error}")
                            }
                        }
                    }
                });
                if self.busy() {
                    ui.spinner();
                    context.request_repaint_after(std::time::Duration::from_millis(40));
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        self.open = window_open;
    }

    fn start(&mut self, work: Work) -> bool {
        if self.busy() {
            return false;
        }
        let (sender, receiver) = mpsc::channel();
        let creating = matches!(&work, Work::Create { .. });
        let worker = std::thread::Builder::new()
            .name("orr-room-project".into())
            .spawn(move || {
                let result = match work {
                    Work::Create { output, seed } => {
                        if output.as_os_str().is_empty()
                            || output.as_os_str().as_encoded_bytes().len() > 4096
                        {
                            Err("output path must contain 1..=4096 native bytes".into())
                        } else {
                            orr_sample::windows_project::create_room_project(&output, &seed)
                                .map(|project| project.root().to_path_buf())
                        }
                    }
                    Work::Open { root } => {
                        if root.as_os_str().is_empty()
                            || root.as_os_str().as_encoded_bytes().len() > 4096
                        {
                            Err("project path must contain 1..=4096 native bytes".into())
                        } else {
                            orr_sample::windows_project::open_room_project(&root)
                                .map(|project| project.root().to_path_buf())
                        }
                    }
                };
                let _ = sender.send(result);
            });
        match worker {
            Ok(_) => {
                self.pending = Some(receiver);
                self.ready = None;
                self.status = if creating {
                    "Creating and validating Room project in background…".into()
                } else {
                    "Validating existing Room project in background…".into()
                };
                true
            }
            Err(error) => {
                self.status = format!("Cannot start Room project worker: {error}");
                false
            }
        }
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String) {
    let response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .char_limit(4096),
    )
    .labelled_by(response.id);
}
