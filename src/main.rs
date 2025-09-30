use chrono::{DateTime, Datelike, Local, Timelike, Weekday};
use eframe::{egui, App, Frame};
use egui::{
    Color32, ColorImage, Context, DragValue, Id, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2,
};
use egui_extras::DatePickerButton;
use notify_rust::Notification;
use rfd::FileDialog;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
enum LoopFrequency {
    Once,
    Daily,
    Weekly,
    Monthly,
    Yearly,
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}
impl LoopFrequency {
    const ALL: [Self; 12] = [
        Self::Once,
        Self::Daily,
        Self::Weekly,
        Self::Monthly,
        Self::Yearly,
        Self::Sunday,
        Self::Monday,
        Self::Tuesday,
        Self::Wednesday,
        Self::Thursday,
        Self::Friday,
        Self::Saturday,
    ];
}
#[derive(Serialize, Deserialize, Clone)]
struct Note {
    id: usize,
    position: Pos2,
    text: String,
    size: Vec2,
}
#[derive(Serialize, Deserialize, Clone)]
struct Todo {
    id: usize,
    position: Pos2,
    text: String,
    due: Option<DateTime<Local>>,
    is_done: bool,
    size: Vec2,
    #[serde(default)]
    loop_freq: LoopFrequency,
    #[serde(default)]
    notified: bool,
}
impl Default for LoopFrequency {
    fn default() -> Self {
        LoopFrequency::Once
    }
}

#[derive(Serialize, Deserialize, Default)]
struct AppState {
    notes: Vec<Note>,
    todos: Vec<Todo>,
    connections: Vec<(usize, usize)>,
    offset: Vec2,
    zoom: f32,
    next_id: usize,
    background_image_path: Option<String>,
    #[serde(skip)]
    connecting_from_id: Option<usize>,
}

impl AppState {
    fn get_item_pos(&self, id: usize) -> Option<Pos2> {
        self.notes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.position + n.size / 2.0)
            .or_else(|| {
                self.todos
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.position + t.size / 2.0)
            })
    }
}

struct EndlessCanvasApp {
    state: Arc<Mutex<AppState>>,
    storage_path: Option<PathBuf>,
    background_texture: Option<TextureHandle>,
}

fn load_image_from_path(path: &Path) -> Result<ColorImage, image::ImageError> {
    let image = image::ImageReader::open(path)?.decode()?;
    let size = [image.width() as _, image.height() as _];
    let image_buffer = image.to_rgba8();
    let pixels = image_buffer.as_flat_samples();
    Ok(ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()))
}

fn get_next_due(current_due: &DateTime<Local>, freq: LoopFrequency) -> DateTime<Local> {
    let now = Local::now();
    let mut next_due = *current_due;
    while next_due <= now {
        next_due = match freq {
            LoopFrequency::Once => next_due,
            LoopFrequency::Daily => next_due + chrono::Duration::days(1),
            LoopFrequency::Weekly => next_due + chrono::Duration::weeks(1),
            LoopFrequency::Monthly => next_due + chrono::Duration::days(30),
            LoopFrequency::Yearly => next_due + chrono::Duration::days(365),
            LoopFrequency::Sunday
            | LoopFrequency::Monday
            | LoopFrequency::Tuesday
            | LoopFrequency::Wednesday
            | LoopFrequency::Thursday
            | LoopFrequency::Friday
            | LoopFrequency::Saturday => {
                let target_weekday = match freq {
                    LoopFrequency::Sunday => Weekday::Sun,
                    LoopFrequency::Monday => Weekday::Mon,
                    LoopFrequency::Tuesday => Weekday::Tue,
                    LoopFrequency::Wednesday => Weekday::Wed,
                    LoopFrequency::Thursday => Weekday::Thu,
                    LoopFrequency::Friday => Weekday::Fri,
                    _ => Weekday::Sat,
                };
                let mut next = next_due + chrono::Duration::days(1);
                while next.weekday() != target_weekday {
                    next = next + chrono::Duration::days(1);
                }
                next
            }
        };
    }
    next_due
}
fn generate_title(text: &str) -> String {
    let title = text
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        " ".to_string()
    } else {
        title
    }
}
#[derive(Clone, Copy)]
struct Transformer {
    offset: Vec2,
    zoom: f32,
}
impl Transformer {
    fn to_screen(&self, pos: Pos2) -> Pos2 {
        pos * self.zoom + self.offset
    }
    fn from_screen(&self, pos: Pos2) -> Pos2 {
        (pos - self.offset) / self.zoom
    }
}

impl EndlessCanvasApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let storage_path = Self::get_storage_path();
        let state = Self::from_disk(storage_path.as_deref()).unwrap_or_default();

        let mut background_texture = None;
        if let Some(path_str) = &state.background_image_path {
            if let Ok(image) = load_image_from_path(Path::new(path_str)) {
                background_texture = Some(cc.egui_ctx.load_texture(
                    "background",
                    image,
                    Default::default(),
                ));
            }
        }

        let app_state = Arc::new(Mutex::new(state));
        let notification_state = Arc::clone(&app_state);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(15));
            let mut state = notification_state.lock().unwrap();
            let now = Local::now();
            for todo in state.todos.iter_mut() {
                if !todo.is_done {
                    if let Some(due_time) = todo.due {
                        if now > due_time && !todo.notified {
                            Notification::new()
                                .summary("Todo Due!")
                                .body(&todo.text)
                                .show()
                                .ok();
                            if todo.loop_freq == LoopFrequency::Once {
                                todo.notified = true;
                            } else {
                                todo.due = Some(get_next_due(&due_time, todo.loop_freq));
                            }
                        }
                    }
                }
            }
        });

        Self {
            state: app_state,
            storage_path,
            background_texture,
        }
    }

    fn get_storage_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "gemini", "endlesscanvas")
            .map(|p| p.data_dir().join("app_state.json"))
    }
    fn from_disk(path: Option<&Path>) -> Option<AppState> {
        let p = path?;
        std::fs::read_to_string(p)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
    }
    fn save_state(&self) {
        if let Some(path) = &self.storage_path {
            if let Ok(json) = serde_json::to_string_pretty(&*self.state.lock().unwrap()) {
                if let Some(p) = path.parent() {
                    _ = std::fs::create_dir_all(p);
                }
                _ = std::fs::write(path, json);
            }
        }
    }
}

//TODO
//+add remove todo or note action
//+run in background for notifications
//+add focus mode
//...

impl App for EndlessCanvasApp {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        let mut state = self.state.lock().unwrap();
        let transformer = Transformer {
            offset: state.offset,
            zoom: state.zoom,
        };
        ctx.request_repaint();

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.background_texture {
                ui.painter().image(
                    texture.id(),
                    ui.max_rect(),
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }

            let response = ui.interact(ui.max_rect(), ui.id(), Sense::click_and_drag());
            if response.dragged() {
                state.offset += response.drag_delta();
            }
            if response.clicked() && state.connecting_from_id.is_some() {
                state.connecting_from_id = None;
            }
            if let Some(hover_pos) = response.hover_pos() {
                ui.input(|i| {
                    let z = i.zoom_delta();
                    if z != 1.0 {
                        let oz = state.zoom;
                        state.zoom = (state.zoom * z).clamp(0.1, 10.0);
                        state.offset = hover_pos - (hover_pos - state.offset) * (state.zoom / oz);
                    }
                });
            }
            let painter = ui.painter();
            for (id1, id2) in state.connections.clone() {
                if let (Some(wp1), Some(wp2)) = (state.get_item_pos(id1), state.get_item_pos(id2)) {
                    painter.line_segment(
                        [transformer.to_screen(wp1), transformer.to_screen(wp2)],
                        Stroke::new(2.0, Color32::from_gray(128)),
                    );
                }
            }
            if let (Some(sid), Some(cpos)) = (state.connecting_from_id, ctx.pointer_interact_pos())
            {
                if let Some(wpos) = state.get_item_pos(sid) {
                    painter.line_segment(
                        [transformer.to_screen(wpos), cpos],
                        Stroke::new(2.0, Color32::LIGHT_BLUE),
                    );
                }
            }

            response.context_menu(|ui| {
                if ui.button("Add Note").clicked() {
                    let np = transformer.from_screen(
                        ui.ctx()
                            .pointer_interact_pos()
                            .unwrap_or(ui.max_rect().center()),
                    );
                    let nid = state.next_id;
                    state.next_id += 1;
                    state.notes.push(Note {
                        id: nid,
                        position: np,
                        text: "New note".to_string(),
                        size: Vec2::new(200.0, 100.0),
                    });
                    ui.close_menu();
                }
                if ui.button("Add Todo").clicked() {
                    let np = transformer.from_screen(
                        ui.ctx()
                            .pointer_interact_pos()
                            .unwrap_or(ui.max_rect().center()),
                    );
                    let nid = state.next_id;
                    state.next_id += 1;
                    state.todos.push(Todo {
                        id: nid,
                        position: np,
                        text: "New todo".to_string(),
                        due: None,
                        is_done: false,
                        size: Vec2::new(200.0, 150.0),
                        loop_freq: LoopFrequency::Once,
                        notified: false,
                    });
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Change Background").clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Image", &["png", "jpg", "jpeg"])
                        .pick_file()
                    {
                        if let Ok(image) = load_image_from_path(&path) {
                            state.background_image_path = Some(path.display().to_string());
                            self.background_texture =
                                Some(ctx.load_texture("background", image, Default::default()));
                        }
                    }
                    ui.close_menu();
                }
            });

            let mut clicked_ch: Option<usize> = None;
            for note in state.notes.iter_mut() {
                let r = egui::Window::new(generate_title(&note.text))
                    .id(Id::new("note").with(note.id))
                    .default_pos(transformer.to_screen(note.position))
                    .default_size(note.size * transformer.zoom)
                    .show(ctx, |ui| {
                        if ui.button("🔗").clicked() {
                            clicked_ch = Some(note.id);
                        }
                        ui.add(egui::TextEdit::multiline(&mut note.text).frame(false));
                    });
                if let Some(r) = r {
                    note.position = transformer.from_screen(r.response.rect.min);
                    note.size = r.response.rect.size() / transformer.zoom;
                }
            }
            for todo in state.todos.iter_mut() {
                let r = egui::Window::new(generate_title(&todo.text))
                    .id(Id::new("todo").with(todo.id))
                    .default_pos(transformer.to_screen(todo.position))
                    .default_size(todo.size * transformer.zoom)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("🔗").clicked() {
                                clicked_ch = Some(todo.id);
                            }
                            ui.checkbox(&mut todo.is_done, "");
                            ui.add(egui::TextEdit::singleline(&mut todo.text).frame(false));
                        });
                        ui.separator();
                        let mut dc = false;
                        let mut td = todo.due.unwrap_or_else(Local::now);
                        ui.horizontal(|ui| {
                            ui.label("Due:");
                            let mut d = td.date_naive();
                            if ui.add(DatePickerButton::new(&mut d)).changed() {
                                if let Some(nt) =
                                    d.and_time(td.time()).and_local_timezone(Local).single()
                                {
                                    td = nt;
                                    dc = true;
                                }
                            }
                            let mut h = td.hour();
                            let mut m = td.minute();
                            if ui
                                .add(DragValue::new(&mut h).speed(1).clamp_range(0..=23))
                                .changed()
                                || ui
                                    .add(DragValue::new(&mut m).speed(1).clamp_range(0..=59))
                                    .changed()
                            {
                                if let (Some(hr), Some(min)) = (td.with_hour(h), td.with_minute(m))
                                {
                                    td = hr.with_minute(min.minute()).unwrap_or(td);
                                    dc = true;
                                }
                            }
                        });
                        if dc {
                            todo.due = Some(td);
                            todo.notified = false;
                        }
                        egui::ComboBox::from_label("Frequency")
                            .selected_text(format!("{:?}", todo.loop_freq))
                            .show_ui(ui, |ui| {
                                for f in LoopFrequency::ALL {
                                    ui.selectable_value(&mut todo.loop_freq, f, format!("{:?}", f));
                                }
                            });
                    });
                if let Some(r) = r {
                    todo.position = transformer.from_screen(r.response.rect.min);
                    todo.size = r.response.rect.size() / transformer.zoom;
                }
            }
            if let Some(cid) = clicked_ch {
                if let Some(sid) = state.connecting_from_id.take() {
                    if sid != cid {
                        state.connections.push((sid, cid));
                    }
                } else {
                    state.connecting_from_id = Some(cid);
                }
            }
        });
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_state();
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "My Test Project in Rust",
        options,
        Box::new(|cc| Box::new(EndlessCanvasApp::new(cc))),
    )
}
