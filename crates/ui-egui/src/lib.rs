//! DeckCraft's egui frontend: a PowerPoint-style UI over `deckcraft-engine`.
//!
//! The UI is thin: every action goes through [`SlideApp::run`], which handles UI commands
//! (views, panes, zoom) here and sends everything else to the engine. The ribbon, menus,
//! shortcuts, the command palette and the control channel all share that entry point.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod backstage;
pub mod canvas;
pub mod control;
pub mod credits;
pub mod dialogs;
pub mod fillui;
pub mod icons;
pub mod keymap;
pub mod media;
pub mod menus;
pub mod panes;
pub mod ribbon;
pub mod selpane;
pub mod show;
pub mod sorter;
pub mod status;
pub mod textures;
pub mod theme;
pub mod thumbs;
pub mod toast;
pub mod uifonts;
pub mod widgets;

use std::sync::mpsc::{Receiver, Sender};

use deckcraft_engine::{Mods, Session, UiRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use control::{ControlRequest, ControlResponse};

pub type ReadFn = Box<dyn Fn(&str) -> Result<Vec<u8>, String>>;
pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;
pub type PickFn = Box<dyn FnMut(&str) -> Option<String>>;
pub type OpenAsyncFn = Box<dyn FnMut(&str)>;
pub type DownloadFn = Box<dyn FnMut(&str, &[u8])>;
/// Files `(name, bytes)` delivered asynchronously by the host (web file picker, dropped files).
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// Platform services injected by the host (desktop or web).
#[derive(Default)]
pub struct Services {
    /// Open-file dialog for a purpose (`open`, `picture`, `audio`, `video`) → path.
    pub pick_open: Option<PickFn>,
    /// Save dialog with a suggested name → path.
    pub pick_save: Option<PickFn>,
    pub read: Option<ReadFn>,
    pub write: Option<WriteFn>,
    /// Asynchronous open dialog (web): the chosen file arrives through `inbox`.
    pub open_async: Option<OpenAsyncFn>,
    /// Hand bytes to the user as a named file (browser download).
    pub download: Option<DownloadFn>,
    pub inbox: Option<Inbox>,
    /// Image on the system clipboard (PNG bytes), when the host can read one.
    pub clipboard_image: Option<Box<dyn FnMut() -> Option<Vec<u8>>>>,
    /// Audio output for media playback (cpal on desktop); without one media plays silently.
    pub audio_out: Option<Box<dyn deckcraft_media::AudioOut>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewMode {
    #[default]
    Normal,
    Outline,
    Sorter,
    NotesPage,
    Reading,
}

/// Persisted UI state.
/// What DeckCraft shows when it starts with nothing to open (File › Settings › General).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Startup {
    /// The File view's Home page, with the recent presentations.
    #[default]
    StartScreen,
    /// The presentation at the top of the recent list.
    LastFile,
    /// A new presentation, as New › Blank Presentation makes it.
    Blank,
}

impl Startup {
    pub const ALL: [Startup; 3] = [Startup::StartScreen, Startup::LastFile, Startup::Blank];

    /// The id `app.startup {open}` takes.
    pub fn id(self) -> &'static str {
        match self {
            Startup::StartScreen => "startScreen",
            Startup::LastFile => "lastFile",
            Startup::Blank => "blank",
        }
    }

    pub fn parse(id: &str) -> Option<Startup> {
        Startup::ALL.into_iter().find(|s| s.id().eq_ignore_ascii_case(id))
    }

    pub fn label(self) -> &'static str {
        match self {
            Startup::StartScreen => "The start screen",
            Startup::LastFile => "The presentation I had open last",
            Startup::Blank => "A new blank presentation",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    pub brightness: theme::Brightness,
    /// The interface theme (File › Settings › Themes), in light or dark per `brightness`.
    pub ui_theme: theme::UiTheme,
    pub custom_theme: theme::CustomTheme,
    /// Custom keyboard shortcuts (File › Settings › Keyboard Shortcuts): only the actions the user changed.
    pub keymap: keymap::Keymap,
    /// File › Settings › General: no sliding or fading in DeckCraft's own interface.
    pub reduce_motion: bool,
    /// File › Settings › General: what DeckCraft shows when it starts with nothing to open.
    pub startup: Startup,
    pub tab: String,
    pub ribbon_collapsed: bool,
    pub view: ViewMode,
    pub notes: bool,
    pub notes_height: f32,
    pub thumbs_width: f32,
    /// Open task pane: `format`, `animation`, `selection`, `comments`, `designer`, `background`.
    pub pane: Option<String>,
    pub pane_width: f32,
    pub ruler: bool,
    pub gridlines: bool,
    pub guides: bool,
    pub smart_guides: bool,
    /// None = fit to window.
    pub zoom: Option<f32>,
    pub sorter_zoom: f32,
    pub recent: Vec<String>,
    pub presenter_view: bool,
    pub ui_scale: f32,
    /// Format Shape pane: tab (`fill`, `effects`, `size`, `picture`, `text`).
    pub format_tab: String,
    pub grayscale: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            brightness: theme::Brightness::Light,
            ui_theme: theme::UiTheme::Standard,
            custom_theme: theme::CustomTheme::default(),
            keymap: keymap::Keymap::default(),
            reduce_motion: false,
            startup: Startup::StartScreen,
            tab: "Home".into(),
            ribbon_collapsed: false,
            view: ViewMode::Normal,
            notes: true,
            notes_height: 52.0,
            thumbs_width: 236.0,
            pane: None,
            pane_width: 300.0,
            ruler: false,
            gridlines: false,
            guides: false,
            smart_guides: true,
            zoom: None,
            sorter_zoom: 1.0,
            recent: vec![],
            presenter_view: true,
            ui_scale: 1.0,
            format_tab: "fill".into(),
            grayscale: false,
        }
    }
}

/// File › Settings › General › Interface size: smallest and largest.
pub const MIN_UI_SCALE: f32 = 0.75;
pub const MAX_UI_SCALE: f32 = 2.0;
/// The largest settings backup `settings.import` reads.
const MAX_SETTINGS_BYTES: usize = 1 << 20;

/// UI-only commands (id, label, shortcut, params).
pub const UI_COMMANDS: &[(&str, &str, Option<&str>, &str)] = &[
    ("view.normal", "Normal", None, "{}"),
    ("view.outline", "Outline View", None, "{}"),
    ("view.sorter", "Slide Sorter", None, "{}"),
    ("view.notesPage", "Notes Page", None, "{}"),
    ("view.reading", "Reading View", None, "{}"),
    ("view.notes", "Notes", None, "{on?: bool}"),
    ("view.ruler", "Ruler", None, "{on?: bool}"),
    ("view.gridlines", "Gridlines", Some("Shift+F9"), "{on?: bool}"),
    ("view.guides", "Guides", Some("Alt+F9"), "{on?: bool}"),
    ("view.zoomIn", "Zoom In", Some("Cmd+="), "{}"),
    ("view.zoomOut", "Zoom Out", Some("Cmd+-"), "{}"),
    ("view.zoom", "Zoom…", None, "{percent}"),
    ("view.fit", "Fit to Window", None, "{}"),
    ("view.pane", "Task Pane", None, "{pane: format|animation|selection|comments|designer|background|null}"),
    ("view.tab", "Ribbon Tab", None, "{tab}"),
    ("view.collapseRibbon", "Collapse the Ribbon", Some("Cmd+F1"), "{}"),
    ("view.dark", "Dark Mode", None, "{on?: bool}"),
    (
        "view.uiTheme",
        "Interface Theme",
        None,
        "{theme?: standard|highContrast|lagoon|sunset|custom, tint?: #rrggbb, accent?: #rrggbb, strength?: 0-1}",
    ),
    ("view.grayscale", "Grayscale", None, "{on?: bool}"),
    ("show.start", "Slide Show", Some("F5"), "{from?: index}"),
    ("show.presenter", "Presenter View", None, "{on?: bool}"),
    ("show.end", "End Show", None, "{}"),
    ("app.home", "Home", None, "{on?}"),
    (
        "app.backstage",
        "File",
        Some("Alt+F"),
        "{page?: home|new|open|info|saveAs|print|export|account|settings, tab?: themes|shortcuts|reset, on?: bool}",
    ),
    ("app.closePresentation", "Close", Some("Cmd+W"), "{}"),
    ("app.print", "Print", Some("Cmd+P"), "{layout?, perPage?, slides?, includeHidden?, frame?}"),
    ("app.about", "About DeckCraft", None, "{}"),
    ("app.palette", "Command Palette", Some("Cmd+Shift+P"), "{}"),
    ("app.preferences", "Preferences…", Some("Cmd+,"), "{}"),
    ("app.openDialog", "Open…", None, "{purpose?}"),
    ("app.insertPictureDialog", "Picture from File…", None, "{}"),
    ("app.insertAudioDialog", "Audio from File…", None, "{}"),
    ("app.insertVideoDialog", "Video from File…", None, "{}"),
    ("app.saveAsDialog", "Save As…", None, "{}"),
    ("app.exportDialog", "Export…", None, "{}"),
    ("app.dialog", "Open Dialog", None, "{id}"),
    ("app.links", "Community Links", None, "{}"),
    ("keys.list", "Keyboard Shortcuts", None, "{} → [{id, label, group, scope, keys, defaults, custom}]"),
    ("keys.set", "Set Keyboard Shortcut", None, "{action, keys: [\"Cmd+Shift+K\"…], replace?: bool}"),
    ("keys.reset", "Reset Keyboard Shortcuts", None, "{action?}"),
    ("app.resetSettings", "Reset All Settings", None, "{}"),
    ("view.interfaceSize", "Interface Size", None, "{scale: 0.75 to 2, 1 = 100%}"),
    ("view.reduceMotion", "Reduce Motion", None, "{on?: bool}"),
    ("app.startup", "When DeckCraft Starts", None, "{open: startScreen|lastFile|blank}"),
    (
        "settings.newPresentation",
        "New Presentation Defaults",
        None,
        "{theme?: built-in name (empty: the default), size?: widescreen|standard, font?: name (empty: the theme's), forgetShape?: bool}",
    ),
    ("settings.export", "Back Up Settings", None, "{path?} → {settings} or {path}"),
    ("settings.import", "Restore Settings", None, "{path? | settings?: what settings.export gave}"),
    ("keys.cheatSheet", "Keyboard Shortcut Cheat Sheet", None, "{} → {text, html}"),
];

pub struct Perf {
    pub frame_ms: f64,
    pub render_ms: f64,
    pub fps: f64,
}

pub struct SlideApp {
    pub session: Session,
    pub services: Services,
    pub ui: UiState,
    pub textures: textures::Textures,
    pub control_rx: Option<Receiver<ControlRequest>>,
    pub dialog: Option<dialogs::Dialog>,
    pub show: Option<show::Show>,
    /// Audio/video playback (editor control bar, slide show, `media.*` commands).
    pub media: media::MediaHost,
    pub status: Option<(String, f64)>,
    /// A brief notice over the bottom of the window (missing fonts on opening a file).
    pub toast: Option<toast::Toast>,
    /// Documents (by uid) whose missing fonts have been mentioned.
    fonts_noted: std::collections::HashSet<u64>,
    pub perf: Perf,
    /// Synthetic input events (control channel `ui.click` etc.), injected one per frame.
    pub synthetic: Vec<egui::Event>,
    /// Canvas screen rect and points-per-slide-point scale (for control & tests).
    pub canvas_rect: Option<egui::Rect>,
    pub canvas_scale: f32,
    pub slide_rect: Option<egui::Rect>,
    /// Where the canvas texture (the slide and the pasteboard drawn around it) is on screen.
    pub canvas_region: Option<egui::Rect>,
    pub palette: Option<(String, usize)>,
    /// Integrated title bar (macOS traffic lights over our chrome).
    pub integrated_titlebar: bool,
    /// The File tab's full-window view (Backstage). Over a presentation it replaces the ribbon,
    /// slides and status bar until Esc or Back; with nothing open it is the start screen. Opening,
    /// creating or closing a presentation leaves it.
    pub backstage: Option<backstage::Backstage>,
    pending_urls: Vec<String>,
    shot_token: u64,
    queued_shots: Vec<(u64, f64, u32)>,
    pending_shots: Vec<(u64, Option<String>, Sender<ControlResponse>, f64)>,
    styled: bool,
    restyle: bool,
    fonts_ready: bool,
    last_time: f64,
    /// When AutoRecover data was last written (seconds, egui time).
    pub last_recovery: f64,
    /// Quit was confirmed (changes saved or discarded): let the window close.
    pub quit_confirmed: bool,
    pub(crate) notes_buf: (u64, usize, String),
    /// Faces borrowed from the font database for text the interface fonts can't draw (#101).
    pub ui_fonts: uifonts::UiFonts,
    /// Thumbnail pane drag (slide index, current drop index).
    pub(crate) thumb_drag: Option<(usize, usize)>,
    /// Selection pane: folded groups, renaming, drag and drop.
    pub selpane: selpane::State,
}

impl SlideApp {
    pub fn new(session: Session, mut services: Services) -> Self {
        let audio_out = services.audio_out.take();
        SlideApp {
            session,
            services,
            ui: UiState::default(),
            textures: textures::Textures::default(),
            control_rx: None,
            dialog: None,
            show: None,
            media: media::MediaHost::new(audio_out),
            status: None,
            toast: None,
            fonts_noted: Default::default(),
            perf: Perf { frame_ms: 0.0, render_ms: 0.0, fps: 0.0 },
            synthetic: vec![],
            canvas_rect: None,
            canvas_scale: 1.0,
            slide_rect: None,
            canvas_region: None,
            palette: None,
            integrated_titlebar: false,
            backstage: None,
            pending_urls: vec![],
            shot_token: 0,
            queued_shots: vec![],
            pending_shots: vec![],
            styled: false,
            restyle: true,
            fonts_ready: false,
            last_time: 0.0,
            last_recovery: 0.0,
            quit_confirmed: false,
            notes_buf: (0, usize::MAX, String::new()),
            ui_fonts: uifonts::UiFonts::default(),
            thumb_drag: None,
            selpane: selpane::State::default(),
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), now_ms()));
    }

    /// Run a command by id (UI commands here, everything else in the engine). Errors show in the
    /// status bar and are returned.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let r = if UI_COMMANDS.iter().any(|c| c.0 == id) {
            self.run_ui(id, &params)
        } else {
            self.session.execute(id, &params).map_err(|e| e.to_string())
        };
        if let Err(e) = &r {
            self.set_status(e.clone());
        }
        self.drain_requests();
        r
    }

    pub(crate) fn docs_key(&self) -> (usize, Option<usize>) {
        (self.session.documents().len(), self.session.active_index())
    }

    /// `view.uiTheme`: the interface theme and the Custom theme's colours. Setting a custom
    /// colour without a theme switches to Custom.
    fn set_ui_theme(&mut self, p: &Value) -> Result<Value, String> {
        let colour = |k: &str| -> Result<Option<[u8; 3]>, String> {
            match p.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_str().and_then(theme::parse_hex).map(Some).ok_or_else(|| format!("`{k}` must be a colour like \"#1f6fb8\"")),
            }
        };
        let (tint, accent) = (colour("tint")?, colour("accent")?);
        let strength = match p.get("strength") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_f64().filter(|s| s.is_finite()).ok_or("`strength` must be a number from 0 to 1")?.clamp(0.0, 1.0) as f32),
        };
        let theme = match p.get("theme").and_then(Value::as_str) {
            Some(id) => Some(theme::UiTheme::from_id(id).ok_or_else(|| format!("unknown interface theme `{id}`"))?),
            None if p.get("theme").is_some_and(|v| !v.is_null()) => return Err("`theme` must be a theme id".into()),
            None => None,
        };
        let custom = &mut self.ui.custom_theme;
        custom.tint = tint.unwrap_or(custom.tint);
        custom.accent = accent.unwrap_or(custom.accent);
        custom.strength = strength.unwrap_or(custom.strength);
        self.ui.ui_theme =
            theme.unwrap_or(if tint.is_some() || accent.is_some() || strength.is_some() { theme::UiTheme::Custom } else { self.ui.ui_theme });
        self.restyle = true;
        let c = self.ui.custom_theme;
        Ok(json!({"theme": self.ui.ui_theme.id(), "tint": theme::hex(c.tint), "accent": theme::hex(c.accent), "strength": c.strength}))
    }

    /// `settings.newPresentation`: File › Settings › New Presentations. Each setting given is
    /// checked and stored in the engine's prefs, which `file.new` reads.
    fn new_presentation_defaults(&mut self, p: &Value) -> Result<Value, String> {
        let prefs = &mut self.session.prefs;
        if let Some(name) = p.get("theme").and_then(Value::as_str).map(str::trim) {
            prefs.new_theme = if name.is_empty() {
                String::new()
            } else {
                let th = deckcraft_model::theme::builtin_themes().into_iter().find(|t| t.name.eq_ignore_ascii_case(name));
                th.map(|t| t.name).ok_or_else(|| format!("no built-in theme is called `{name}`"))?
            };
        }
        if let Some(size) = p.get("size").and_then(Value::as_str) {
            prefs.new_size = match size.to_ascii_lowercase().as_str() {
                "standard" => "standard".into(),
                "widescreen" => String::new(),
                _ => return Err(format!("`size` must be widescreen or standard, not `{size}`")),
            };
        }
        if let Some(font) = p.get("font").and_then(Value::as_str).map(str::trim) {
            if font.len() > 128 {
                return Err("`font` is too long to be a font name".into());
            }
            prefs.new_font = font.to_string();
        }
        if p.get("forgetShape").and_then(Value::as_bool) == Some(true) {
            prefs.default_shape = None;
        }
        Ok(json!({
            "theme": prefs.new_theme,
            "size": if prefs.new_size == "standard" { "standard" } else { "widescreen" },
            "font": prefs.new_font,
            "defaultShape": prefs.default_shape.is_some(),
        }))
    }

    /// What a settings backup holds: the interface's settings and the engine's prefs. The recent
    /// files stay out: they name documents on this computer.
    fn settings_backup(&self) -> Result<Value, String> {
        let mut ui = self.ui.clone();
        ui.recent.clear();
        let ui = serde_json::to_value(&ui).map_err(|e| e.to_string())?;
        let prefs = serde_json::to_value(&self.session.prefs).map_err(|e| e.to_string())?;
        Ok(json!({"deckcraft": "settings", "version": 1, "ui": ui, "prefs": prefs}))
    }

    /// `settings.export {path?}`: the backup, written to `path` when one is given.
    fn export_settings(&mut self, p: &Value) -> Result<Value, String> {
        let backup = self.settings_backup()?;
        let Some(path) = p.get("path").and_then(Value::as_str) else { return Ok(json!({"settings": backup})) };
        let bytes = serde_json::to_vec_pretty(&backup).map_err(|e| e.to_string())?;
        let write = self.services.write.as_mut().ok_or("this window can't write files")?;
        write(path, &bytes)?;
        Ok(json!({"path": path}))
    }

    /// `settings.import {path | settings}`: every setting from a backup. The recent files stay as
    /// they are. A file that isn't a backup changes nothing.
    fn import_settings(&mut self, p: &Value) -> Result<Value, String> {
        const NOT_A_BACKUP: &str = "that isn't a DeckCraft settings backup";
        let backup: Value = match (p.get("settings"), p.get("path").and_then(Value::as_str)) {
            (Some(v), _) => v.clone(),
            (None, Some(path)) => {
                let read = self.services.read.as_ref().ok_or("this window can't read files")?;
                let bytes = read(path)?;
                if bytes.len() > MAX_SETTINGS_BYTES {
                    return Err(NOT_A_BACKUP.into());
                }
                serde_json::from_slice(&bytes).map_err(|_| NOT_A_BACKUP.to_string())?
            }
            (None, None) => return Err("give the backup's `path`, or its `settings`".into()),
        };
        if backup.get("deckcraft").and_then(Value::as_str) != Some("settings") {
            return Err(NOT_A_BACKUP.into());
        }
        let part = |k: &str| backup.get(k).cloned().unwrap_or_else(|| json!({}));
        let ui: UiState = serde_json::from_value(part("ui")).map_err(|e| format!("{NOT_A_BACKUP}: {e}"))?;
        let prefs: deckcraft_engine::Prefs = serde_json::from_value(part("prefs")).map_err(|e| format!("{NOT_A_BACKUP}: {e}"))?;
        let recent = std::mem::take(&mut self.ui.recent);
        self.ui = UiState { recent, ..ui };
        self.ui.keymap.sanitize();
        self.session.prefs = prefs;
        self.restyle = true;
        Ok(json!({"restored": true}))
    }

    /// File › Settings › Backup and Reset › Back up settings…: a file the user picks (desktop) or
    /// a download (web).
    pub fn backup_settings(&mut self) {
        const NAME: &str = "DeckCraft settings.json";
        if let Some(pick) = self.services.pick_save.as_mut() {
            let Some(path) = pick(NAME) else { return };
            if self.run("settings.export", json!({"path": path})).is_ok() {
                self.set_status(format!("Backed up your settings to {path}"));
            }
            return;
        }
        match self.settings_backup().and_then(|v| serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())) {
            Ok(bytes) => {
                self.download(NAME, Ok(bytes));
            }
            Err(e) => self.set_status(e),
        }
    }

    /// File › Settings › Backup and Reset › Restore from a backup…
    pub fn restore_settings(&mut self) {
        let Some(pick) = self.services.pick_open.as_mut() else {
            self.set_status("Restoring settings needs a file picker, which this window doesn't have.");
            return;
        };
        let Some(path) = pick("settings") else { return };
        if self.run("settings.import", json!({"path": path})).is_ok() {
            self.set_status("Restored your settings from the backup");
        }
    }

    /// File › Settings › Keyboard Shortcuts › Save cheat sheet…: a page with every shortcut,
    /// opened in the browser to print (desktop) or downloaded (web).
    pub fn save_cheat_sheet(&mut self) {
        const NAME: &str = "DeckCraft keyboard shortcuts.html";
        let (_, html) = self.ui.keymap.cheat_sheet();
        let Some(pick) = self.services.pick_save.as_mut() else {
            self.download(NAME, Ok(html.into_bytes()));
            return;
        };
        let Some(path) = pick(NAME) else { return };
        let written = match self.services.write.as_mut() {
            Some(write) => write(&path, html.as_bytes()),
            None => Err("this window can't write files".into()),
        };
        match written {
            Ok(()) => {
                self.open_url(&format!("file:///{}", path.replace('\\', "/").trim_start_matches('/')));
                self.set_status(format!("Saved the cheat sheet to {path}; print it from your browser"));
            }
            Err(e) => self.set_status(e),
        }
    }

    /// File › Settings › General › When DeckCraft starts: called once the files given on the
    /// command line are open. With nothing open, opens the last presentation or a blank one; the
    /// start screen needs nothing.
    pub fn startup(&mut self) {
        if self.session.active().is_some() {
            return;
        }
        match self.ui.startup {
            Startup::StartScreen => {}
            Startup::LastFile => {
                if let Some(path) = self.ui.recent.first().cloned()
                    && let Err(e) = self.open_path(&path)
                {
                    self.set_status(format!("Couldn't reopen {path}: {e}"));
                }
            }
            Startup::Blank => {
                let _ = self.run("file.new", json!({}));
            }
        }
    }

    /// Ctrl+C, Ctrl+X and Ctrl+V arrive as Copy, Cut and Paste events as well as key presses: when
    /// the user gave their key to another action, or gave the action other keys, the event is
    /// dropped and the key press does what the custom keys say.
    fn clipboard_key_moved(&self, e: &egui::Event) -> bool {
        let key = match e {
            egui::Event::Copy => egui::Key::C,
            egui::Event::Cut => egui::Key::X,
            _ => egui::Key::V,
        };
        let c = keymap::Combo::from_egui(key, egui::Modifiers::COMMAND);
        self.ui.keymap.hit(keymap::Scope::Editor, &c) != keymap::Hit::Pass
    }

    /// The interface colours for the current theme and light/dark choice.
    pub fn tokens(&self) -> theme::Tokens {
        theme::Tokens::build(self.ui.ui_theme, self.ui.brightness, &self.ui.custom_theme)
    }

    /// The File tab's view is open over a presentation (not just the start screen with nothing
    /// open).
    pub fn backstage_open(&self) -> bool {
        self.backstage.is_some() && self.session.active().is_some()
    }

    /// Opens the File tab's view on `page` (keeping its state when it is already open).
    pub fn open_backstage(&mut self, page: backstage::Page) {
        let key = self.docs_key();
        match &mut self.backstage {
            Some(b) => b.go(page),
            None => self.backstage = Some(backstage::Backstage::new(page, key)),
        }
    }

    /// Back to the presentation (no effect on the start screen, where nothing is open).
    pub fn close_backstage(&mut self) {
        if self.session.active().is_some() {
            self.backstage = None;
        }
    }

    /// Keeps the File tab's view in step with the open presentations: it closes when one is
    /// opened, created or closed (the result shows), and it is the start screen when none is open.
    fn sync_backstage(&mut self) {
        let key = self.docs_key();
        let has_doc = self.session.active().is_some();
        let stale = self.backstage.as_ref().is_some_and(|b| b.docs != key);
        if has_doc && stale {
            self.backstage = None;
        } else if !has_doc && (stale || self.backstage.is_none()) {
            self.backstage = Some(backstage::Backstage::new(backstage::Page::Home, key));
        }
    }

    /// File › Close and Ctrl+W: closes the active presentation, asking first when it has unsaved
    /// changes (Save, Don't Save, Cancel), as PowerPoint does.
    pub fn close_presentation(&mut self) {
        match self.session.active() {
            None => {}
            Some(d) if d.is_dirty() => self.dialog = Some(dialogs::Dialog::new("close")),
            Some(_) => {
                let _ = self.run("file.close", json!({}));
            }
        }
    }

    /// The file name a save or export suggests: the presentation's title with `ext`.
    fn suggested_name(&self, ext: &str) -> String {
        let name = self.session.active().map(|d| d.title()).unwrap_or_else(|| "Presentation".into());
        format!("{name}.{ext}")
    }

    /// Exports the active presentation as `fmt` (`pdf`, `png`, `pngAll`, `jpeg`, `pptx`,
    /// `deckcraft`, `outline`) through `file.export`: the save picker on the desktop, a download
    /// on the web. `extra` adds `file.export` params (the PDF's layout, slides…). The
    /// presentation keeps its own file. Returns whether a file was written or handed over.
    pub fn export_as(&mut self, fmt: &str, extra: Value) -> bool {
        let Some(slide) = self.session.active().map(|d| d.selection.slide) else { return false };
        let (ext, format, all) = match fmt {
            "pdf" => ("pdf", "pdf", false),
            "png" => ("png", "png", false),
            "pngAll" => ("png", "png", true),
            "jpeg" | "jpg" => ("jpg", "jpeg", false),
            "pptx" => ("pptx", "pptx", false),
            "outline" | "txt" => ("txt", "outline", false),
            "deckcraft" => ("deckcraft", "deckcraft", false),
            other => {
                self.set_status(format!("DeckCraft can't export “{other}”"));
                return false;
            }
        };
        let suggested = self.suggested_name(ext);
        if let Some(pick) = self.services.pick_save.as_mut() {
            let Some(path) = pick(&suggested) else { return false };
            let mut params = json!({"path": path, "format": format, "all": all});
            if let (Some(o), Some(x)) = (params.as_object_mut(), extra.as_object()) {
                o.extend(x.clone());
            }
            return match self.run("file.export", params) {
                Ok(_) => {
                    self.set_status(format!("Exported {path}"));
                    true
                }
                Err(_) => false,
            };
        }
        let Some(doc) = self.session.active().map(|d| d.doc.clone()) else { return false };
        let bytes = match format {
            "png" | "jpeg" => {
                let img = deckcraft_render::render_slide(&doc, slide, &deckcraft_render::RenderOpts { scale: 2.0, ..Default::default() });
                Ok(if format == "png" { img.to_png() } else { img.to_jpeg(92) })
            }
            "pdf" => deckcraft_engine::cmd::file::pdf_bytes(&doc, &extra),
            other => deckcraft_engine::cmd::file::save_bytes(&doc, other),
        };
        self.download(&suggested, bytes)
    }

    /// Hands `bytes` to the browser as a download (web); false (with the reason in the status
    /// bar) when there are no bytes or no way to download.
    fn download(&mut self, name: &str, bytes: deckcraft_engine::Result<Vec<u8>>) -> bool {
        match (bytes, self.services.download.as_mut()) {
            (Ok(b), Some(dl)) => {
                dl(name, &b);
                true
            }
            (Err(e), _) => {
                self.set_status(e.to_string());
                false
            }
            (Ok(_), None) => {
                self.set_status("Saving a copy needs a file picker, which this window doesn't have.");
                false
            }
        }
    }

    /// File › Print: DeckCraft prints through a PDF. On the desktop the PDF (with the Print page's
    /// layout, slides and options) is written to the temporary folder and opened in the system's
    /// PDF viewer, which has the printers; on the web it downloads.
    pub fn print_pdf(&mut self, params: Value) -> bool {
        let Some((title, doc)) = self.session.active().map(|d| (d.title(), d.doc.clone())) else { return false };
        let mut params = if params.is_object() { params } else { json!({}) };
        if self.services.pick_save.is_none() {
            // Web: the browser hands the PDF to the user.
            let bytes = deckcraft_engine::cmd::file::pdf_bytes(&doc, &params);
            return self.download(&format!("{title}.pdf"), bytes);
        }
        let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let path = std::env::temp_dir().join(format!("{safe} (print).pdf")).to_string_lossy().to_string();
        if let Some(o) = params.as_object_mut() {
            o.insert("path".into(), json!(path));
            o.insert("format".into(), json!("pdf"));
        }
        match self.run("file.export", params) {
            Ok(_) => {
                self.open_url(&format!("file:///{}", path.replace('\\', "/").trim_start_matches('/')));
                self.set_status("Opened the presentation as a PDF to print");
                true
            }
            Err(_) => false,
        }
    }

    fn flag(&mut self, params: &Value, cur: bool) -> bool {
        params.get("on").and_then(Value::as_bool).unwrap_or(!cur)
    }

    fn run_ui(&mut self, id: &str, p: &Value) -> Result<Value, String> {
        match id {
            "view.normal" => self.ui.view = ViewMode::Normal,
            "view.outline" => self.ui.view = ViewMode::Outline,
            "view.sorter" => self.ui.view = ViewMode::Sorter,
            "view.notesPage" => self.ui.view = ViewMode::NotesPage,
            "view.reading" => self.start_show(self.session.active().map(|d| d.selection.slide).unwrap_or(0), true),
            "view.notes" => self.ui.notes = self.flag(p, self.ui.notes),
            "view.ruler" => self.ui.ruler = self.flag(p, self.ui.ruler),
            "view.gridlines" => self.ui.gridlines = self.flag(p, self.ui.gridlines),
            "view.guides" => self.ui.guides = self.flag(p, self.ui.guides),
            "view.zoomIn" | "view.zoomOut" => {
                let cur = self.ui.zoom.unwrap_or(self.canvas_scale);
                let steps = [0.1, 0.25, 0.33, 0.5, 0.66, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
                let next = if id == "view.zoomIn" {
                    steps.iter().copied().find(|s| *s > cur + 0.01).unwrap_or(4.0)
                } else {
                    steps.iter().rev().copied().find(|s| *s < cur - 0.01).unwrap_or(0.1)
                };
                self.ui.zoom = Some(next);
            }
            "view.zoom" => {
                let pct = p.get("percent").and_then(Value::as_f64).ok_or("missing `percent`")?;
                self.ui.zoom = Some((pct as f32 / 100.0).clamp(0.1, 4.0));
            }
            "view.fit" => self.ui.zoom = None,
            "view.pane" => {
                let pane = p.get("pane").and_then(Value::as_str).map(String::from);
                self.ui.pane =
                    if pane.is_some() && pane == self.ui.pane && p.get("toggle").and_then(Value::as_bool).unwrap_or(false) { None } else { pane };
                if let Some(tab) = p.get("tab").and_then(Value::as_str) {
                    self.ui.format_tab = tab.to_string();
                }
            }
            "view.tab" => {
                let tab = p.get("tab").and_then(Value::as_str).ok_or("missing `tab`")?;
                self.ui.tab = tab.to_string();
                self.ui.ribbon_collapsed = false;
            }
            "view.collapseRibbon" => self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed,
            "view.dark" => {
                let on = self.flag(p, self.ui.brightness == theme::Brightness::Dark);
                self.ui.brightness = if on { theme::Brightness::Dark } else { theme::Brightness::Light };
                self.restyle = true;
            }
            "view.uiTheme" => return self.set_ui_theme(p),
            "keys.list" => return Ok(self.ui.keymap.list()),
            "keys.set" => return self.ui.keymap.set(p),
            "keys.reset" => return self.ui.keymap.reset(p),
            "keys.cheatSheet" => {
                let (text, html) = self.ui.keymap.cheat_sheet();
                return Ok(json!({"text": text, "html": html}));
            }
            "view.interfaceSize" => {
                let scale = p.get("scale").and_then(Value::as_f64).filter(|s| s.is_finite()).ok_or("`scale` must be a number, 1 = 100%")?;
                self.ui.ui_scale = (scale as f32).clamp(MIN_UI_SCALE, MAX_UI_SCALE);
                return Ok(json!({"scale": self.ui.ui_scale}));
            }
            "view.reduceMotion" => {
                self.ui.reduce_motion = self.flag(p, self.ui.reduce_motion);
                self.restyle = true;
            }
            "app.startup" => {
                let id = p.get("open").and_then(Value::as_str).unwrap_or("");
                self.ui.startup = Startup::parse(id).ok_or_else(|| format!("`open` must be startScreen, lastFile or blank, not `{id}`"))?;
            }
            "settings.newPresentation" => return self.new_presentation_defaults(p),
            "settings.export" => return self.export_settings(p),
            "settings.import" => return self.import_settings(p),
            "app.resetSettings" => {
                // Every setting back to how DeckCraft ships; the recent files are a list of the
                // user's documents, not a setting, so they stay.
                let recent = std::mem::take(&mut self.ui.recent);
                self.ui = UiState { recent, ..UiState::default() };
                self.session.prefs = deckcraft_engine::Prefs::default();
                self.restyle = true;
            }
            "view.grayscale" => self.ui.grayscale = self.flag(p, self.ui.grayscale),
            "show.start" => {
                let from = p.get("from").and_then(Value::as_u64).map(|v| v as usize).unwrap_or(0);
                self.start_show(from, false);
            }
            "show.presenter" => self.ui.presenter_view = self.flag(p, self.ui.presenter_view),
            "show.end" => {
                self.show = None;
                self.media.stop_owned(media::Owner::Show);
            }
            "app.home" => {
                let on = self.flag(p, self.backstage_open());
                if on {
                    self.open_backstage(backstage::Page::Home);
                } else {
                    self.close_backstage();
                }
            }
            "app.backstage" => {
                if !p.get("on").and_then(Value::as_bool).unwrap_or(true) {
                    self.close_backstage();
                    return Ok(json!({"page": self.backstage.as_ref().map(|b| b.page.id())}));
                }
                let page = match p.get("page").and_then(Value::as_str) {
                    Some(id) => backstage::Page::parse(id).ok_or_else(|| format!("unknown File page `{id}`"))?,
                    None => backstage::Page::Home,
                };
                if page.needs_doc() && self.session.active().is_none() {
                    return Err(format!("File › {} needs an open presentation", page.label()));
                }
                let tab = match p.get("tab").and_then(Value::as_str) {
                    Some(id) => Some(backstage::SettingsTab::parse(id).ok_or_else(|| format!("unknown Settings tab `{id}`"))?),
                    None => None,
                };
                self.open_backstage(page);
                if let (Some(tab), Some(b)) = (tab, self.backstage.as_mut()) {
                    b.open_tab(tab);
                }
                return Ok(json!({"page": page.id()}));
            }
            "app.closePresentation" => self.close_presentation(),
            "app.print" => {
                if self.session.active().is_none() {
                    return Err("nothing to print: no presentation is open".into());
                }
                return Ok(json!({"printed": self.print_pdf(p.clone())}));
            }
            "app.about" => self.dialog = Some(dialogs::Dialog::new("about")),
            "app.palette" => self.palette = Some((String::new(), 0)),
            "app.preferences" => self.dialog = Some(dialogs::Dialog::new("preferences")),
            "app.openDialog" => self.pick_and_open(),
            "app.insertPictureDialog" => self.pick_and_insert("picture", "insert.picture"),
            "app.insertAudioDialog" => self.pick_and_insert("audio", "insert.audio"),
            "app.insertVideoDialog" => self.pick_and_insert("video", "insert.video"),
            "app.saveAsDialog" => self.save_as_dialog("deckcraft"),
            "app.exportDialog" => self.dialog = Some(dialogs::Dialog::new("export")),
            "app.dialog" => {
                let d = p.get("id").and_then(Value::as_str).ok_or("missing `id`")?;
                self.dialog = Some(dialogs::Dialog::new(d));
            }
            "app.links" => {
                use deckcraft_engine::links::*;
                return Ok(json!({"discord": DISCORD, "website": WEBSITE, "apps": APPS, "appPage": APP_PAGE, "github": GITHUB}));
            }
            _ => return Err(format!("unknown UI command `{id}`")),
        }
        Ok(Value::Null)
    }

    pub fn start_show(&mut self, from: usize, reading: bool) {
        self.media.stop_owned(media::Owner::Editor);
        if let Some(d) = self.session.active() {
            self.show = Some(show::Show::new(&d.doc, from, reading, self.ui.presenter_view && !reading));
        }
    }

    fn pick_and_open(&mut self) {
        if let Some(pick) = self.services.pick_open.as_mut() {
            if let Some(path) = pick("open") {
                let _ = self.open_path(&path);
            }
        } else if let Some(f) = self.services.open_async.as_mut() {
            f("open");
        }
    }

    pub fn open_path(&mut self, path: &str) -> Result<(), String> {
        self.session.execute("file.open", &json!({"path": path})).map_err(|e| e.to_string())?;
        self.ui.recent.retain(|p| p != path);
        self.ui.recent.insert(0, path.to_string());
        self.ui.recent.truncate(12);
        self.ui.view = ViewMode::Normal;
        Ok(())
    }

    fn pick_and_insert(&mut self, purpose: &str, cmd: &str) {
        if let Some(pick) = self.services.pick_open.as_mut() {
            if let Some(path) = pick(purpose) {
                let r = match self.services.read.as_ref().map(|r| r(&path)) {
                    Some(Ok(bytes)) => {
                        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        self.session.execute(cmd, &json!({"name": name, "data": deckcraft_engine::cmd::base64_encode(&bytes)}))
                    }
                    _ => self.session.execute(cmd, &json!({"path": path})),
                };
                if let Err(e) = r {
                    self.set_status(e.to_string());
                }
            }
        } else if let Some(f) = self.services.open_async.as_mut() {
            f(purpose);
        }
    }

    pub fn save_as_dialog(&mut self, format: &str) {
        let name = self.session.active().map(|d| d.title()).unwrap_or_else(|| "Presentation".into());
        let ext = if format == "pptx" { "pptx" } else { "deckcraft" };
        let suggested = format!("{name}.{ext}");
        if let Some(pick) = self.services.pick_save.as_mut() {
            if let Some(path) = pick(&suggested) {
                match self.session.execute("file.saveAs", &json!({"path": path})) {
                    Ok(_) => {
                        self.set_status(format!("Saved {path}"));
                        self.ui.recent.retain(|p| *p != path);
                        self.ui.recent.insert(0, path);
                    }
                    Err(e) => self.set_status(e.to_string()),
                }
            }
        } else if let Some(dl) = self.services.download.as_mut()
            && let Some(d) = self.session.active()
        {
            match deckcraft_engine::cmd::file::save_bytes(&d.doc, ext) {
                Ok(bytes) => dl(&suggested, &bytes),
                Err(e) => self.set_status(e.to_string()),
            }
        }
    }

    /// Save every presentation with unsaved changes (asking for a location where needed).
    /// Returns false if one wasn't saved (cancelled or failed).
    pub fn save_all(&mut self) -> bool {
        let dirty: Vec<usize> = self.session.documents().iter().enumerate().filter(|(_, d)| d.is_dirty()).map(|(i, _)| i).collect();
        for i in dirty {
            self.session.set_active(i);
            self.save();
            if self.session.documents().get(i).is_some_and(|d| d.is_dirty()) {
                return false;
            }
        }
        true
    }

    pub fn save(&mut self) {
        if self.session.active().and_then(|d| d.path.clone()).is_some() {
            match self.session.execute("file.save", &json!({})) {
                Ok(_) => self.set_status("Saved"),
                Err(e) => self.set_status(e.to_string()),
            }
        } else {
            self.save_as_dialog("deckcraft");
        }
    }

    /// Act on requests commands left for the UI.
    fn drain_requests(&mut self) {
        for r in std::mem::take(&mut self.session.ui_requests) {
            match r {
                UiRequest::Dialog { id, params } => {
                    let mut d = dialogs::Dialog::new(&id);
                    d.params = params;
                    self.dialog = Some(d);
                }
                UiRequest::Pane { id } => self.ui.pane = Some(id),
                UiRequest::StartShow { from } => self.start_show(from, false),
                UiRequest::Message { text } => self.set_status(text),
                UiRequest::PickFile { purpose } => {
                    if purpose == "saveAs" {
                        self.save_as_dialog("deckcraft");
                    }
                }
                UiRequest::Media { action, shape, ms } => {
                    let owner = if self.show.is_some() { media::Owner::Show } else { media::Owner::Editor };
                    if let Some(doc) = self.session.active().map(|d| d.doc.clone()) {
                        self.media.request(&doc, &action, shape, ms, owner);
                    }
                }
            }
        }
    }

    fn drain_inbox(&mut self) {
        let Some(inbox) = self.services.inbox.clone() else { return };
        let files: Vec<(String, Vec<u8>)> = inbox.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
        for (name, bytes) in files {
            self.open_or_insert_bytes(&name, &bytes);
        }
    }

    /// A dropped or picked file: presentations open, media is inserted.
    pub fn open_or_insert_bytes(&mut self, name: &str, bytes: &[u8]) {
        let lower = name.to_ascii_lowercase();
        let data = deckcraft_engine::cmd::base64_encode(bytes);
        let ct = deckcraft_engine::cmd::insert::content_type(name, bytes);
        let cmd = if lower.ends_with(".deckcraft")
            || lower.ends_with(".slidecraft")
            || lower.ends_with(".pptx")
            || lower.ends_with(".potx")
            || lower.ends_with(".ppsx")
        {
            "file.openBytes"
        } else if ct.starts_with("image/") {
            "insert.picture"
        } else if ct.starts_with("audio/") {
            "insert.audio"
        } else if ct.starts_with("video/") {
            "insert.video"
        } else if lower.ends_with(".txt") || lower.ends_with(".md") {
            "slide.fromOutline"
        } else {
            self.set_status(format!("{name}: DeckCraft can't open this kind of file"));
            return;
        };
        let p = if cmd == "slide.fromOutline" { json!({"text": String::from_utf8_lossy(bytes)}) } else { json!({"name": name, "data": data}) };
        if self.session.active().is_none() && cmd != "file.openBytes" {
            let _ = self.session.execute("file.new", &json!({}));
        }
        if let Err(e) = self.session.execute(cmd, &p) {
            self.set_status(e.to_string());
        }
    }

    /// Per-frame logic before layout: styles, control requests, screenshots, shortcuts.
    pub fn logic(&mut self, ctx: &egui::Context) {
        let scale = self.ui.ui_scale.clamp(0.5, 3.0);
        if (ctx.zoom_factor() - scale).abs() > 1e-3 {
            ctx.set_zoom_factor(scale);
        }
        if !self.styled {
            self.ui.keymap.sanitize();
            theme::install_fonts_with(ctx, &self.ui_fonts.faces);
            self.styled = true;
            self.restyle = true;
        } else {
            self.fonts_ready = true;
        }
        if self.restyle {
            theme::apply(ctx, &self.tokens());
            // Reduce motion: no animated opening, hovering or scrolling in the interface.
            let calm = self.ui.reduce_motion;
            ctx.all_styles_mut(|s| {
                s.animation_time = if calm { 0.0 } else { egui::Style::default().animation_time };
                s.scroll_animation = if calm { egui::style::ScrollAnimation::none() } else { egui::style::ScrollAnimation::default() };
            });
            self.restyle = false;
        }
        let now = ctx.input(|i| i.time);
        let dt = now - self.last_time;
        if dt > 0.0 {
            self.perf.fps = self.perf.fps * 0.9 + (1.0 / dt).min(240.0) * 0.1;
        }
        self.last_time = now;
        self.drain_control(ctx);
        if self.fonts_ready {
            self.drain_inbox();
        }
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        }
        if self.fonts_ready {
            self.note_ui_text(ctx);
        }
        self.collect_screenshots(ctx);
        self.issue_screenshots(ctx);
        // Closing the window with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_confirmed && self.session.documents().iter().any(|d| d.is_dirty()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show = None;
            if self.dialog.as_ref().is_none_or(|d| d.id != "quit") {
                self.dialog = Some(dialogs::Dialog::new("quit"));
            }
        }
        // AutoRecover: unsaved presentations are written to the recovery folder periodically.
        if self.session.recovery_dir.is_some() && now - self.last_recovery > (self.session.prefs.recovery_minutes * 60.0).max(10.0) {
            self.last_recovery = now;
            if self.session.documents().iter().any(|d| d.is_dirty())
                && let Err(e) = self.session.execute("file.recovery.save", &json!({}))
            {
                self.set_status(format!("Couldn't save AutoRecover information: {e}"));
            }
        }
        // On the web, dropped files can only be read asynchronously: the host reads them and
        // delivers them through `Services::inbox`.
        #[cfg(not(target_arch = "wasm32"))]
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped file".into());
            match f.bytes() {
                Ok(bytes) => self.open_or_insert_bytes(&name, &bytes),
                Err(e) => self.set_status(e),
            }
        }
    }

    /// Borrow faces for interface text in scripts the interface fonts lack (#101): the open
    /// presentations' names and text, recent files, and what is typed or composed this frame.
    fn note_ui_text(&mut self, ctx: &egui::Context) {
        let typed: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) | egui::Event::Paste(t) => Some(t.as_str()),
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text)) => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        });
        self.ui_fonts.note(ctx, &typed);
        for d in self.session.documents() {
            self.ui_fonts.note(ctx, &d.title());
        }
        for r in &self.ui.recent {
            self.ui_fonts.note(ctx, r);
        }
        if let Some(st) = self.session.active() {
            self.ui_fonts.note_doc(ctx, st);
        }
        self.ui_fonts.apply(ctx);
    }

    /// An input method's event for the slide's text (#101): the preedit is shown in the text and
    /// replaced on each update; the commit replaces it as one undo step.
    fn ime_event(&mut self, e: &egui::ImeEvent) {
        let r = match e {
            egui::ImeEvent::Preedit { text, active_range_chars } => {
                self.session.compose_text(text, active_range_chars.as_ref().map(|r| (r.start, r.end)))
            }
            egui::ImeEvent::Commit(text) => self.session.commit_text(text),
            #[allow(deprecated)]
            egui::ImeEvent::Disabled => {
                self.session.end_composition();
                Ok(Value::Null)
            }
            _ => Ok(Value::Null),
        };
        if let Err(e) = r {
            self.set_status(e.to_string());
        }
    }

    /// Inject synthetic events (one pointer event per frame).
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        if self.synthetic.is_empty() {
            return;
        }
        let n = match self.synthetic.first() {
            Some(egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }) => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        if let Some(egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. }) = self.synthetic.first() {
            raw.events.push(egui::Event::PointerMoved(*p));
        }
        let n = n.min(self.synthetic.len());
        raw.events.extend(self.synthetic.drain(..n));
    }

    /// Keyboard and clipboard for the slide (when no text field has focus).
    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.backstage.as_ref().is_some_and(|b| b.recording.is_some()) {
            return;
        }
        if ctx.memory(|m| m.focused().is_some()) || self.dialog.as_ref().is_some_and(|d| d.modal()) || self.palette.is_some() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        if self.backstage.is_some() {
            // The File tab: its own keys (Esc goes back, arrows walk the side menu) and app
            // shortcuts (New, Open, Command Palette…); typing, pasting or deleting would edit the
            // hidden presentation.
            for e in events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                    let mods = Mods { shift: modifiers.shift, alt: modifiers.alt, cmd: modifiers.command || modifiers.ctrl };
                    if backstage::key(self, key, mods) || !(mods.cmd || mods.alt) {
                        continue;
                    }
                    match self.ui.keymap.hit(keymap::Scope::Editor, &keymap::Combo::from_egui(key, modifiers)) {
                        // Only app actions: the presentation is hidden behind the File view.
                        keymap::Hit::Run(id) if id.starts_with("app.") => {
                            let _ = self.run(&id, json!({}));
                            continue;
                        }
                        keymap::Hit::Run(_) | keymap::Hit::Swallow => continue,
                        keymap::Hit::Pass => {}
                    }
                    if menus::ui_shortcut(self, key, mods) {
                        continue;
                    }
                    // Engine shortcuts that don't touch the hidden slide (Ctrl+N).
                    if mods.cmd
                        && matches!(key, egui::Key::N)
                        && let Err(e) = self.session.key(key_name(key), mods)
                    {
                        self.set_status(e.to_string());
                    }
                }
            }
            self.drain_requests();
            return;
        }
        for e in events {
            match e {
                egui::Event::Text(t) => {
                    if self.session.active().is_some_and(|d| d.selection.text.is_some() || d.selection.shapes.len() == 1)
                        && let Err(e) = self.session.type_text(&t)
                    {
                        self.set_status(e.to_string());
                    }
                }
                egui::Event::Ime(ime) => self.ime_event(&ime),
                egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) if self.clipboard_key_moved(&e) => {}
                egui::Event::Copy => {
                    if let Ok(v) = self.session.execute("edit.copy", &json!({}))
                        && let Some(t) = v.get("text").and_then(Value::as_str)
                    {
                        ctx.copy_text(t.to_string());
                    }
                }
                egui::Event::Cut => {
                    if let Ok(v) = self.session.execute("edit.cut", &json!({}))
                        && let Some(t) = v.get("text").and_then(Value::as_str)
                    {
                        ctx.copy_text(t.to_string());
                    }
                }
                egui::Event::Paste(text) => {
                    // An image on the clipboard (screenshots, copied pictures) becomes a picture.
                    let img = if text.is_empty() { self.services.clipboard_image.as_mut().and_then(|f| f()) } else { None };
                    let r = match img {
                        Some(png) => self
                            .session
                            .execute("insert.picture", &json!({"name": "Pasted image.png", "data": deckcraft_engine::cmd::base64_encode(&png)})),
                        None => self.session.execute("edit.paste", &json!({"text": text})),
                    };
                    if let Err(e) = r {
                        self.set_status(e.to_string());
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    let mods = Mods { shift: modifiers.shift, alt: modifiers.alt, cmd: modifiers.command || modifiers.ctrl };
                    match self.ui.keymap.hit(keymap::Scope::Editor, &keymap::Combo::from_egui(key, modifiers)) {
                        keymap::Hit::Run(id) => {
                            let _ = self.run(&id, json!({}));
                            continue;
                        }
                        keymap::Hit::Swallow => continue,
                        keymap::Hit::Pass => {}
                    }
                    if !self.session.tool.drawing_freeform() && menus::ui_shortcut(self, key, mods) {
                        continue;
                    }
                    let name = key_name(key);
                    match self.session.key(name, mods) {
                        Ok(_) => {}
                        Err(e) => self.set_status(e.to_string()),
                    }
                }
                _ => {}
            }
        }
        self.drain_requests();
    }

    /// Lay out the whole window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            ctx.request_repaint();
            return;
        }
        let t0 = now_ms();
        let t = theme::Tokens::get(&ctx);
        self.tick_media(&ctx);
        if self.show.is_some() {
            show::ui(self, ui);
            self.perf.frame_ms = now_ms() - t0;
            return;
        }
        self.sync_backstage();
        self.keyboard(&ctx);
        self.sync_backstage();
        ribbon::title_bar(self, ui);
        if self.backstage.is_none() && self.session.active().is_some() {
            ribbon::show(self, ui);
            status::status_bar(self, ui);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.chrome)).show(ui, |ui| {
            if self.backstage.is_some() {
                // File tab: the Backstage view replaces everything below the title bar.
                backstage::show(self, ui);
                return;
            }
            if self.session.active().is_none() {
                return;
            }
            match self.ui.view {
                ViewMode::Sorter => sorter::show(self, ui),
                ViewMode::NotesPage => sorter::notes_page(self, ui),
                ViewMode::Outline | ViewMode::Normal | ViewMode::Reading => {
                    if self.ui.pane.is_some() {
                        panes::show(self, ui);
                    }
                    if self.ui.view == ViewMode::Outline {
                        sorter::outline_pane(self, ui);
                    } else {
                        thumbs::show(self, ui);
                    }
                    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
                        if self.ui.notes {
                            status::notes_pane(self, ui);
                        }
                        canvas::show(self, ui);
                    });
                }
            }
        });
        toast::note_missing_fonts(self);
        toast::show(self, &ctx);
        dialogs::show(self, &ctx);
        menus::palette(self, &ctx);
        for url in std::mem::take(&mut self.pending_urls) {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        self.drain_requests();
        self.perf.frame_ms = now_ms() - t0;
    }

    /// Media clocks and video textures; the editor's media stops when its slide is left.
    fn tick_media(&mut self, ctx: &egui::Context) {
        if !self.media.any_active() {
            self.session.media_status.clear();
            return;
        }
        if self.show.is_none()
            && let Some(d) = self.session.active()
        {
            let here: Vec<deckcraft_model::ShapeId> = if d.selection.target == deckcraft_engine::Target::Slides {
                media::slide_media(&d.doc, d.selection.slide).into_iter().map(|m| m.0).collect()
            } else {
                vec![]
            };
            let gone: Vec<_> =
                self.media.ids().into_iter().filter(|id| !here.contains(id) && self.media.owner(*id) == Some(media::Owner::Editor)).collect();
            for id in gone {
                self.media.stop(id);
            }
        }
        self.media.tick(ctx, &mut self.session);
    }

    pub fn open_url(&mut self, url: &str) {
        self.pending_urls.push(url.to_string());
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path } => {
                    self.shot_token += 1;
                    let token = self.shot_token;
                    self.queued_shots.push((token, now_ms() + 120.0, 0));
                    self.pending_shots.push((token, path, reply, now_ms() + 8000.0));
                }
            }
        }
        self.control_rx = Some(rx);
    }

    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        let now = now_ms();
        self.queued_shots.retain_mut(|(token, at, frames)| {
            *frames += 1;
            if now >= *at && *frames >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
                false
            } else {
                true
            }
        });
        if !self.queued_shots.is_empty() || !self.pending_shots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_shots.is_empty() {
            return;
        }
        let events: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                        Some((token, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in events {
            if let Some(i) = self.pending_shots.iter().position(|(t, ..)| *t == token) {
                let (_, path, reply, _) = self.pending_shots.remove(i);
                let _ = reply.send(control::save_screenshot(self, &image, path.as_deref()));
            }
        }
        let now = now_ms();
        self.pending_shots.retain(|(_, _, reply, deadline)| {
            if now < *deadline {
                return true;
            }
            let _ = reply.send(json!({"ok": false, "error": "no frame was presented (screen locked or window hidden); use ui.render"}));
            false
        });
    }
}

/// Engine key name for an egui key.
pub fn key_name(key: egui::Key) -> &'static str {
    use egui::Key::*;
    match key {
        Enter => "Enter",
        Escape => "Escape",
        Backspace => "Backspace",
        Delete => "Delete",
        Tab => "Tab",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        ArrowUp => "Up",
        ArrowDown => "Down",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        Space => "Space",
        F2 => "F2",
        F5 => "F5",
        F7 => "F7",
        other => other.name(),
    }
}

pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_now()
    }
}

#[cfg(target_arch = "wasm32")]
fn js_now() -> f64 {
    js_sys::Date::now()
}

#[cfg(test)]
mod ime_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn title_text(app: &SlideApp) -> String {
        let d = app.session.active().unwrap();
        let sh = &d.doc.slides[0].shapes[0];
        sh.text.as_ref().map(|t| t.paragraphs.iter().map(|p| p.text()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
    }

    #[test]
    fn opening_a_file_with_missing_fonts_shows_a_toast_once() {
        use egui_kittest::kittest::Queryable;
        let mut p = deckcraft_model::Presentation::default();
        let slide = std::sync::Arc::make_mut(&mut p.slides[0]);
        let mut body = deckcraft_model::text::TextBody::from_text("Hello");
        body.paragraphs[0].runs[0].props.font = Some("Qqq Nonexistent Sans".into());
        slide.shapes[0].text = Some(body);
        let data = deckcraft_engine::cmd::base64_encode(&deckcraft_engine::cmd::file::save_bytes(&p, "pptx").unwrap());
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            SlideApp::new(Session::with_new(), Services::default()),
        );
        harness.run_steps(2);
        assert!(harness.state().toast.is_none(), "a new presentation has nothing to say");
        harness.state_mut().session.execute("file.openBytes", &json!({"name": "a.pptx", "data": data})).unwrap();
        harness.run_steps(2);
        let text = harness.state().toast.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        assert!(text.starts_with("Qqq Nonexistent Sans isn't installed"), "{text}");
        assert!(harness.query_by_label_contains("Qqq Nonexistent Sans isn't installed").is_some());
        // Closed, it stays closed for that document.
        harness.get_by_label("Close").click();
        harness.run_steps(2);
        assert!(harness.state().toast.is_none());
        harness.run_steps(2);
        assert!(harness.state().toast.is_none());
    }

    /// #17: Home created a new blank presentation instead of going to the start screen.
    #[test]
    fn home_shows_the_start_screen_over_the_open_presentation() {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            SlideApp::new(Session::with_new(), Services::default()),
        );
        // The app keeps requesting repaints, so step frames instead of run().
        harness.run_steps(2);
        let title = harness.state().session.active().unwrap().doc.slides[0].shapes[0].id.0;
        harness.state_mut().session.execute("text.edit", &json!({"id": title})).unwrap();

        harness.state_mut().run("app.home", json!({})).unwrap();
        harness.run_steps(2);
        assert!(harness.state().backstage_open());
        assert_eq!(harness.state().session.documents().len(), 1, "Home doesn't create a presentation");

        // Typing on the start screen doesn't reach the presentation behind it; Escape goes back.
        harness.event(egui::Event::Text("a".into()));
        harness.key_press(egui::Key::Backspace);
        harness.run_steps(2);
        assert_eq!(title_text(harness.state()), "");
        assert!(!harness.state().session.active().unwrap().is_dirty());
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        assert!(!harness.state().backstage_open());

        // Home again toggles; picking a theme on the start screen (a new presentation) leaves it.
        harness.state_mut().run("app.home", json!({})).unwrap();
        harness.run_steps(2);
        assert!(harness.state().backstage_open());
        harness.state_mut().run("app.home", json!({})).unwrap();
        assert!(!harness.state().backstage_open());
        harness.state_mut().run("app.home", json!({"on": true})).unwrap();
        harness.state_mut().run("file.new", json!({})).unwrap();
        harness.run_steps(2);
        assert!(!harness.state().backstage_open(), "a new presentation leaves Home");
        assert_eq!(harness.state().session.documents().len(), 2);
    }

    /// #268: Ctrl+Backspace, Ctrl+Delete, Ctrl+arrows and Ctrl+Shift+arrows in text, through real
    /// egui key events (Windows keys).
    #[test]
    fn ctrl_word_keys_edit_text() {
        use egui::{Key, Modifiers};
        let mut session = Session::with_new();
        session.mac_keys = false;
        session.prefs.autocorrect = false;
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            SlideApp::new(session, Services::default()),
        );
        harness.run_steps(2);
        let title = harness.state().session.active().unwrap().doc.slides[0].shapes[0].id.0;
        harness.state_mut().session.execute("text.edit", &json!({"id": title})).unwrap();
        harness.event(egui::Event::Text("Hello, world.  Foo bar".into()));
        harness.run_steps(1);
        assert_eq!(title_text(harness.state()), "Hello, world.  Foo bar");
        let caret = |h: &egui_kittest::Harness<'_, SlideApp>| h.state().session.active().unwrap().selection.text.clone().unwrap();
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;

        // Ctrl+Left stops at every word and punctuation run, as in PowerPoint.
        let mut stops = vec![];
        for _ in 0..6 {
            harness.key_press_modifiers(ctrl, Key::ArrowLeft);
            harness.run_steps(1);
            stops.push(caret(&harness).caret.1);
        }
        assert_eq!(stops, vec![19, 15, 12, 7, 5, 0]);
        harness.key_press_modifiers(ctrl, Key::ArrowRight);
        harness.run_steps(1);
        assert_eq!(caret(&harness).caret, (0, 5));

        // Ctrl+Shift+Right selects to the next stops.
        harness.key_press_modifiers(ctrl_shift, Key::ArrowRight);
        harness.key_press_modifiers(ctrl_shift, Key::ArrowRight);
        harness.run_steps(1);
        let t = caret(&harness);
        assert_eq!((t.anchor, t.caret), ((0, 5), (0, 12)));

        // Ctrl+Delete deletes the next word from the selection's start; Ctrl+Backspace the one
        // before the caret.
        harness.key_press_modifiers(ctrl, Key::Delete);
        harness.run_steps(1);
        assert_eq!(title_text(harness.state()), "Helloworld.  Foo bar");
        harness.key_press(Key::End);
        harness.key_press_modifiers(ctrl, Key::Backspace);
        harness.run_steps(1);
        assert_eq!(title_text(harness.state()), "Helloworld.  Foo ");

        // Ctrl+Up/Down go by paragraph, Ctrl+Home/End to the text's ends.
        harness.key_press(Key::Enter);
        harness.event(egui::Event::Text("Next".into()));
        harness.key_press_modifiers(ctrl, Key::ArrowUp);
        harness.run_steps(1);
        assert_eq!(caret(&harness).caret, (1, 0));
        harness.key_press_modifiers(ctrl, Key::ArrowUp);
        harness.run_steps(1);
        assert_eq!(caret(&harness).caret, (0, 0));
        harness.key_press_modifiers(ctrl, Key::ArrowDown);
        harness.run_steps(1);
        assert_eq!(caret(&harness).caret, (1, 0));
        harness.key_press_modifiers(ctrl_shift, Key::Home);
        harness.run_steps(1);
        let t = caret(&harness);
        assert_eq!((t.anchor, t.caret), ((1, 0), (0, 0)));

        // Ctrl+Shift+> (egui reports the Period key) grows the selected text's font.
        let size = |h: &mut egui_kittest::Harness<'_, SlideApp>| h.state_mut().session.execute("format.state", &json!({})).unwrap()["size"].as_f64();
        let before = size(&mut harness);
        harness.key_press_modifiers(ctrl_shift, Key::Period);
        harness.run_steps(1);
        let after = size(&mut harness).unwrap();
        assert!(after > before.unwrap_or(0.0), "{before:?} → {after}");
        harness.key_press_modifiers(ctrl, Key::OpenBracket);
        harness.run_steps(1);
        assert!(size(&mut harness).unwrap_or(f64::MAX) < after, "Ctrl+[ shrinks");
    }

    #[test]
    fn home_needs_an_open_presentation() {
        let mut app = SlideApp::new(Session::new(), Services::default());
        app.run("app.home", json!({})).unwrap();
        assert!(!app.backstage_open(), "with nothing open the start screen shows anyway");
    }
    fn canvas_harness() -> egui_kittest::Harness<'static, SlideApp> {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            SlideApp::new(Session::with_new(), Services::default()),
        );
        harness.run_steps(2);
        let app = harness.state_mut();
        // Zoomed out, the canvas shows plenty of pasteboard around the slide.
        app.ui.zoom = Some(0.4);
        app.session.prefs.snap_to_grid = false;
        app.session.prefs.smart_guides = false;
        harness
    }

    fn selected(app: &SlideApp) -> Vec<u64> {
        app.session.active().unwrap().selection.shapes.iter().map(|i| i.0 as u64).collect()
    }

    fn shape_x(app: &SlideApp, id: u64) -> f64 {
        let st = app.session.active().unwrap();
        st.shape(deckcraft_model::ShapeId(id as u32)).and_then(|s| s.xfrm).unwrap().x
    }

    /// #270: PowerPoint's Normal view draws objects off the slide on the area around it, where
    /// they can be selected and moved.
    #[test]
    fn off_slide_objects_show_on_the_pasteboard_and_can_be_selected_and_moved() {
        let mut harness = canvas_harness();
        let off =
            harness.state_mut().session.execute("shape.insert", &json!({"preset": "rect", "rect": [1000.0, 100.0, 120.0, 80.0]})).unwrap()["id"]
                .as_u64()
                .unwrap();
        harness.state_mut().session.execute("edit.select", &json!({"ids": []})).unwrap();
        harness.run_steps(3);

        let app = harness.state();
        let (slide, region, k) = (app.slide_rect.unwrap(), app.canvas_region.unwrap(), app.canvas_scale);
        // The texture reaches past the slide's right edge to the shape's, and no further left/up.
        assert!(region.max.x >= slide.min.x + 1120.0 * k - 1.0, "{region:?} {slide:?}");
        assert!((region.min.x - slide.min.x).abs() < 1.0 && (region.min.y - slide.min.y).abs() < 1.0, "{region:?} {slide:?}");
        let centre = slide.min + egui::vec2(1060.0 * k, 140.0 * k);
        assert!(app.canvas_rect.unwrap().contains(centre));

        // Drawn at full strength on the pasteboard (when a GPU renderer is available here).
        if let Ok(img) = harness.render() {
            let px = |p: egui::Pos2| img.get_pixel(p.x as u32, p.y as u32).0;
            let shape = px(centre);
            let board = px(slide.min + egui::vec2(1060.0 * k, 400.0 * k));
            let doc = &harness.state().session.active().unwrap().doc;
            let accent = doc.masters[0].theme.colors.get(deckcraft_color::SchemeSlot::Accent1);
            let close = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 4;
            assert!(close(shape[0], accent.r) && close(shape[1], accent.g) && close(shape[2], accent.b), "{shape:?} vs {accent:?}");
            assert_ne!(shape, board);
        }

        // Click selects it.
        harness.hover_at(centre);
        harness.run_steps(1);
        harness.drag_at(centre);
        harness.run_steps(1);
        harness.drop_at(centre);
        harness.run_steps(2);
        assert_eq!(selected(harness.state()), vec![off]);

        // Drag moves it, further off the slide.
        let to = centre + egui::vec2(40.0, 0.0);
        harness.hover_at(centre);
        harness.run_steps(1);
        harness.drag_at(centre);
        harness.run_steps(1);
        for i in 1..=4 {
            harness.hover_at(centre + egui::vec2(10.0 * i as f32, 0.0));
            harness.run_steps(1);
        }
        harness.drop_at(to);
        harness.run_steps(2);
        let x = shape_x(harness.state(), off);
        assert!((x - (1000.0 + 40.0 / k as f64)).abs() < 1.5, "{x}");
        // The pasteboard followed it.
        let (slide, region) = (harness.state().slide_rect.unwrap(), harness.state().canvas_region.unwrap());
        assert!(region.max.x >= slide.min.x + (x as f32 + 120.0) * k - 1.0, "{region:?}");

        // Idle frames don't render the canvas again.
        let n = harness.state().textures.canvas_renders;
        harness.run_steps(3);
        assert_eq!(harness.state().textures.canvas_renders, n);
    }

    /// A slide with nothing off it renders just the slide, as before.
    #[test]
    fn the_canvas_renders_only_the_slide_when_nothing_is_off_it() {
        let mut harness = canvas_harness();
        harness.run_steps(3);
        let app = harness.state();
        let (slide, region) = (app.slide_rect.unwrap(), app.canvas_region.unwrap());
        assert!((region.min - slide.min).length() < 0.5 && (region.max - slide.max).length() < 1.0, "{region:?} {slide:?}");
        let n = app.textures.canvas_renders;
        harness.run_steps(3);
        assert_eq!(harness.state().textures.canvas_renders, n, "an unchanged slide isn't rendered again");
    }
}
