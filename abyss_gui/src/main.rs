//! AbyssC — the windowed front-end.
//!
//! A second front-end over `archive_engine`, peer to the CLI. The engine does
//! all the work on a worker thread; this thread only draws the window and polls
//! a shared [`Progress`] counter, so the UI never stalls behind a long crunch.

// Windowed app: no console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod archive_kind;
mod browser;
mod single_instance;
mod theme;
mod update;

use archive_kind::ArchiveKind;
use browser::{Activated, BrowseRow, Location, RowKind};
use single_instance::{Inbox, Instance};
use update::UpdateInfo;

use archive_engine::{CodecOptions, Format, Listing, Progress};
use iced::widget::{
    button, column, container, pick_list, progress_bar, row, scrollable, slider, text, text_input,
    Space,
};
use iced::{Alignment, Element, Length, Size, Subscription, Task};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub fn main() -> iced::Result {
    let launch = parse_launch();

    // A multi-select "Compress with AbyssC" fires this exe once per file. Make
    // the first one the window and forward the rest into it (one archive).
    let inbox: Inbox = Arc::new(Mutex::new(VecDeque::new()));
    let ipc_enabled = match single_instance::acquire(&launch.forward_paths()) {
        Instance::Forwarded => return Ok(()), // handed our path to the primary; done.
        Instance::Primary(listener) => {
            single_instance::serve(listener, inbox.clone());
            true
        }
        Instance::Standalone => false,
    };

    let boot_inbox = inbox.clone();
    iced::application("AbyssC — compression from the depths", Abyss::update, Abyss::view)
        .theme(|_| theme::abyss())
        .subscription(Abyss::subscription)
        .font(include_bytes!("../assets/DejaVuSans.ttf").as_slice())
        .default_font(iced::Font::with_name("DejaVu Sans"))
        .window(iced::window::Settings {
            size: Size::new(1000.0, 700.0),
            min_size: Some(Size::new(840.0, 620.0)),
            icon: window_icon(),
            ..Default::default()
        })
        .run_with(move || (Abyss::boot(launch, boot_inbox.clone(), ipc_enabled), startup_task()))
}

/// The window/taskbar icon, decoded from the embedded `.ico`.
fn window_icon() -> Option<iced::window::Icon> {
    iced::window::icon::from_file_data(include_bytes!("../assets/AbyssC.ico"), None).ok()
}

/// Work kicked off the moment the app launches: a one-shot update check, run on
/// a blocking thread so the window appears instantly regardless of the network.
fn startup_task() -> Task<Message> {
    Task::perform(
        async { tokio::task::spawn_blocking(|| update::check().ok().flatten()).await.ok().flatten() },
        Message::UpdateChecked,
    )
}

/// How the app was asked to open — used by the Windows context-menu verbs.
#[derive(Clone, Debug)]
enum Launch {
    Normal,
    Compress(PathBuf),
    Extract(PathBuf),
    Browse(PathBuf),
}

impl Launch {
    /// Paths to forward to an already-running instance. Only *compress* launches
    /// accumulate (multi-select → one archive); other intents open their own window.
    fn forward_paths(&self) -> Vec<PathBuf> {
        match self {
            Launch::Compress(p) => vec![p.clone()],
            _ => Vec::new(),
        }
    }
}

/// Parse `--compress|--extract|--browse <path>`, or infer from a bare path.
/// Anything unrecognized opens the app normally.
fn parse_launch() -> Launch {
    let mut args = std::env::args_os().skip(1);
    let Some(first) = args.next() else { return Launch::Normal };
    let first = first.to_string_lossy().into_owned();

    let with_path = |a: Option<std::ffi::OsString>| a.map(PathBuf::from);

    match first.as_str() {
        "--compress" | "-c" => with_path(args.next()).map(Launch::Compress).unwrap_or(Launch::Normal),
        "--extract" | "-x" => with_path(args.next()).map(Launch::Extract).unwrap_or(Launch::Normal),
        "--browse" | "-b" => with_path(args.next()).map(Launch::Browse).unwrap_or(Launch::Normal),
        _ => {
            // A bare path: archives open for extraction, everything else to compress.
            let path = PathBuf::from(first);
            if !path.exists() {
                Launch::Normal
            } else if !path.is_dir() && Format::from_path(&path).is_some() {
                Launch::Extract(path)
            } else {
                Launch::Compress(path)
            }
        }
    }
}

// --- State -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Compress,
    Extract,
    Browse,
}

/// What the bottom band reports.
enum Status {
    Idle,
    Running,
    Done(String),
    Failed(String),
}

/// A running engine operation on a worker thread.
struct Job {
    progress: Arc<Progress>,
    outcome: Arc<Mutex<Option<Result<String, String>>>>,
    /// When the job began — drives the "sealing" phase timer once input is
    /// fully consumed but the codec is still finalizing.
    started: Instant,
}

struct Abyss {
    mode: Mode,

    // Compress side.
    inputs: Vec<PathBuf>,
    kind: ArchiveKind,
    level: i32,
    threads: i32,
    output: Option<PathBuf>,
    output_is_auto: bool,

    // Extract side.
    archive: Option<PathBuf>,
    archive_format: Option<Format>,
    listing: Option<Listing>,
    dest: Option<PathBuf>,

    // Browse / Commander side.
    location: Location,
    rows: Vec<BrowseRow>,
    selected: Option<usize>,
    browse_error: Option<String>,
    /// Name of a member currently being drawn out of an archive to open.
    opening: Option<String>,
    /// Last activated row + when, for detecting a double-click to open.
    last_click: Option<(usize, Instant)>,

    // Shared.
    job: Option<Job>,
    status: Status,
    fraction: f32,
    update: Option<UpdateInfo>,

    // Inter-instance: paths forwarded by other launches (multi-select compress).
    incoming: Inbox,
    ipc_enabled: bool,
}

impl Abyss {
    fn new() -> Self {
        let location = browser::initial();
        let rows = browser::rows_for(&location);
        Self {
            mode: Mode::Compress,
            inputs: Vec::new(),
            kind: ArchiveKind::Zstd,
            level: ArchiveKind::Zstd.default_level(),
            threads: 0,
            output: None,
            output_is_auto: true,
            archive: None,
            archive_format: None,
            listing: None,
            dest: None,
            location,
            rows,
            selected: None,
            browse_error: None,
            opening: None,
            last_click: None,
            job: None,
            status: Status::Idle,
            fraction: 0.0,
            update: None,
            incoming: Arc::new(Mutex::new(VecDeque::new())),
            ipc_enabled: false,
        }
    }

    /// Build the app, wire the cross-instance inbox, and apply the launch intent.
    fn boot(launch: Launch, incoming: Inbox, ipc_enabled: bool) -> Self {
        let mut app = Self::new();
        app.incoming = incoming;
        app.ipc_enabled = ipc_enabled;
        match launch {
            Launch::Normal => {}
            Launch::Compress(path) => {
                app.mode = Mode::Compress;
                app.add_inputs(vec![path]);
            }
            Launch::Extract(path) => {
                app.mode = Mode::Extract;
                app.load_archive(path);
            }
            Launch::Browse(path) => {
                app.mode = Mode::Browse;
                app.navigate_to_dropped(path);
            }
        }
        app
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Running)
    }
}

// --- Messages --------------------------------------------------------------

#[derive(Debug, Clone)]
enum Message {
    ModeSelected(Mode),

    // Compress.
    AddFiles,
    AddFolder,
    FilesPicked(Vec<PathBuf>),
    FolderPicked(Option<PathBuf>),
    RemoveInput(usize),
    ClearInputs,
    KindSelected(ArchiveKind),
    LevelChanged(i32),
    ThreadsChanged(i32),
    OutputEdited(String),
    BrowseOutput,
    OutputChosen(Option<PathBuf>),

    // Extract.
    OpenArchive,
    ArchivePicked(Option<PathBuf>),
    ChooseDest,
    DestPicked(Option<PathBuf>),

    // Browse / Commander.
    BrowseActivate(usize),
    BrowseUp,
    BrowseHome,
    BrowseRefresh,
    BrowseToCompress,
    BrowseExtractHere,

    // Shared.
    Start,
    Tick,
    FileDropped(PathBuf),
    /// An archive member finished extracting and was handed to the OS to open.
    FileOpened(Result<(), String>),

    // Update prompt.
    UpdateChecked(Option<UpdateInfo>),
    OpenRelease,
    DismissUpdate,

    // Paths forwarded from another launched instance (multi-select compress).
    PollExternal,
}

// --- Update ----------------------------------------------------------------

impl Abyss {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ModeSelected(mode) => {
                if !self.busy() {
                    self.mode = mode;
                    self.status = Status::Idle;
                    self.fraction = 0.0;
                }
            }

            Message::AddFiles => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_files()
                            .await
                            .map(|handles| handles.iter().map(|h| h.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::FilesPicked,
                );
            }
            Message::FilesPicked(paths) => {
                if !paths.is_empty() {
                    self.add_inputs(paths);
                }
            }
            Message::AddFolder => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_folder()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::FolderPicked,
                );
            }
            Message::FolderPicked(path) => {
                if let Some(path) = path {
                    self.add_inputs(vec![path]);
                }
            }
            Message::RemoveInput(i) => {
                if i < self.inputs.len() {
                    self.inputs.remove(i);
                    self.refresh_suggested_output();
                }
            }
            Message::ClearInputs => {
                self.inputs.clear();
                self.refresh_suggested_output();
            }
            Message::KindSelected(kind) => {
                self.kind = kind;
                self.level = kind.default_level();
                if self.output_is_auto {
                    self.refresh_suggested_output();
                } else if let Some(out) = &self.output {
                    // Re-point the extension if the user hasn't taken manual control.
                    self.output = Some(reext(out, kind.extension()));
                }
            }
            Message::LevelChanged(level) => self.level = level,
            Message::ThreadsChanged(threads) => self.threads = threads.max(0),
            Message::OutputEdited(s) => {
                self.output_is_auto = false;
                self.output = if s.trim().is_empty() { None } else { Some(PathBuf::from(s)) };
            }
            Message::BrowseOutput => {
                let name = self.suggested_file_name();
                let dir = self.inputs.first().and_then(|p| p.parent()).map(PathBuf::from);
                return Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new().set_file_name(name);
                        if let Some(dir) = dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog.save_file().await.map(|h| h.path().to_path_buf())
                    },
                    Message::OutputChosen,
                );
            }
            Message::OutputChosen(path) => {
                if let Some(path) = path {
                    self.output = Some(path);
                    self.output_is_auto = false;
                }
            }

            Message::OpenArchive => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_file()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::ArchivePicked,
                );
            }
            Message::ArchivePicked(path) => {
                if let Some(path) = path {
                    self.load_archive(path);
                }
            }
            Message::ChooseDest => {
                let dir = self.archive.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
                return Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new();
                        if let Some(dir) = dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog.pick_folder().await.map(|h| h.path().to_path_buf())
                    },
                    Message::DestPicked,
                );
            }
            Message::DestPicked(path) => {
                if let Some(path) = path {
                    self.dest = Some(path);
                }
            }

            Message::BrowseActivate(i) => {
                let now = Instant::now();
                let double = matches!(
                    self.last_click,
                    Some((j, t)) if j == i && now.duration_since(t) < Duration::from_millis(450)
                );
                self.last_click = Some((i, now));

                if let Some(row) = self.rows.get(i).cloned() {
                    // Files don't navigate: a single click selects, a double click
                    // opens. Folders/archives/drives keep their one-click behavior.
                    if row.kind == RowKind::File {
                        self.selected = Some(i);
                        if double {
                            return self.open_file_row(&row);
                        }
                    } else {
                        match browser::activate(&self.location, &row) {
                            Activated::Go(loc) => self.set_location(loc),
                            Activated::Stay => self.selected = Some(i),
                            Activated::Error(e) => self.browse_error = Some(e),
                        }
                    }
                }
            }
            Message::BrowseUp => {
                if let Some(loc) = browser::up(&self.location) {
                    self.set_location(loc);
                }
            }
            Message::BrowseHome => self.set_location(Location::Drives),
            Message::BrowseRefresh => {
                let here = self.location.clone();
                self.set_location(here);
            }
            Message::BrowseToCompress => {
                if let Some(path) = self.selected_fs_path() {
                    self.add_inputs(vec![path]);
                    self.mode = Mode::Compress;
                    self.status = Status::Idle;
                }
            }
            Message::BrowseExtractHere => {
                if let Location::Archive { path, .. } = &self.location {
                    let path = path.clone();
                    self.load_archive(path);
                    self.mode = Mode::Extract;
                }
            }

            Message::Start => self.start_job(),
            Message::Tick => self.poll_job(),
            Message::FileDropped(path) => match self.mode {
                Mode::Compress => self.add_inputs(vec![path]),
                Mode::Extract => self.load_archive(path),
                Mode::Browse => self.navigate_to_dropped(path),
            },
            Message::FileOpened(result) => {
                self.opening = None;
                if let Err(e) = result {
                    self.browse_error = Some(e);
                }
            }

            Message::PollExternal => {
                let drained: Vec<PathBuf> = match self.incoming.lock() {
                    Ok(mut q) => q.drain(..).collect(),
                    Err(_) => Vec::new(),
                };
                if !drained.is_empty() && !self.busy() {
                    self.mode = Mode::Compress;
                    self.add_inputs(drained);
                    self.status = Status::Idle;
                }
            }

            Message::UpdateChecked(info) => self.update = info,
            Message::OpenRelease => {
                if let Some(u) = &self.update {
                    open_url(&u.url);
                }
                self.update = None;
            }
            Message::DismissUpdate => self.update = None,
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let drops = iced::event::listen_with(|event, _status, _id| match event {
            iced::Event::Window(iced::window::Event::FileDropped(path)) => {
                Some(Message::FileDropped(path))
            }
            _ => None,
        });

        let mut subs = vec![drops];
        if self.busy() {
            subs.push(iced::time::every(Duration::from_millis(60)).map(|_| Message::Tick));
        }
        if self.ipc_enabled {
            // Pick up files forwarded by sibling instances. Idle when none arrive.
            subs.push(
                iced::time::every(Duration::from_millis(250)).map(|_| Message::PollExternal),
            );
        }
        Subscription::batch(subs)
    }
}

// --- Update helpers --------------------------------------------------------

impl Abyss {
    fn add_inputs(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if !self.inputs.contains(&p) {
                self.inputs.push(p);
            }
        }
        self.refresh_suggested_output();
    }

    fn suggested_file_name(&self) -> String {
        let stem = self
            .inputs
            .first()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".to_string());
        format!("{stem}{}", self.kind.extension())
    }

    fn refresh_suggested_output(&mut self) {
        if !self.output_is_auto {
            return;
        }
        self.output = self.inputs.first().map(|first| {
            let dir = first.parent().map(PathBuf::from).unwrap_or_default();
            dir.join(self.suggested_file_name())
        });
    }

    fn load_archive(&mut self, path: PathBuf) {
        match Format::from_path(&path) {
            Some(format) => {
                self.archive_format = Some(format);
                self.listing = archive_engine::list(&path, format).ok();
                self.dest = path.parent().map(PathBuf::from);
                self.archive = Some(path);
                self.status = Status::Idle;
                self.fraction = 0.0;
            }
            None => {
                self.archive = Some(path);
                self.archive_format = None;
                self.listing = None;
                self.status = Status::Failed(
                    "Unknown extension — the Abyss does not recognize this form.".to_string(),
                );
            }
        }
    }

    fn set_location(&mut self, loc: Location) {
        self.location = loc;
        self.rows = browser::rows_for(&self.location);
        self.selected = None;
        self.browse_error = None;
    }

    /// The full on-disk path of the selected filesystem row, if it is one.
    fn selected_fs_path(&self) -> Option<PathBuf> {
        let Location::Fs(dir) = &self.location else { return None };
        let row = self.rows.get(self.selected?)?;
        matches!(row.kind, RowKind::File | RowKind::Dir | RowKind::Archive)
            .then(|| dir.join(&row.name))
    }

    /// Resolve a path dropped onto the Commander into a sensible location.
    fn navigate_to_dropped(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.set_location(Location::Fs(path));
        } else if Format::from_path(&path).is_some() {
            // Open the archive at its root via the filesystem row machinery.
            if let Some(dir) = path.parent() {
                self.set_location(Location::Fs(dir.to_path_buf()));
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if let Some(i) = self.rows.iter().position(|r| {
                        r.kind == RowKind::Archive && r.name == name
                    }) {
                        if let Activated::Go(loc) =
                            browser::activate(&self.location, &self.rows[i].clone())
                        {
                            self.set_location(loc);
                        }
                    }
                }
            }
        } else if let Some(dir) = path.parent() {
            self.set_location(Location::Fs(dir.to_path_buf()));
        }
    }

    /// Open an activated file row. A file on disk is handed straight to the OS;
    /// a file inside an archive is drawn out to a temp file (just that one member)
    /// on a worker thread, then opened — the archive is never fully unpacked.
    fn open_file_row(&mut self, row: &BrowseRow) -> Task<Message> {
        match &self.location {
            Location::Fs(dir) => {
                open_path(&dir.join(&row.name));
                Task::none()
            }
            Location::Archive { path, format, inner, .. } => {
                let src = path.clone();
                let format = *format;
                let member = format!("{inner}{}", row.name);
                let name = row.name.clone();
                self.opening = Some(name.clone());
                self.browse_error = None;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || open_archive_member(&src, format, &member, &name))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    Message::FileOpened,
                )
            }
            Location::Drives => Task::none(),
        }
    }

    /// The output path with the format's extension enforced. If the user typed a
    /// bare or wrong-suffixed name (e.g. `test` while ZSTD is selected), it gains
    /// the dropdown's extension; a bare name is anchored to the source folder.
    fn finalized_output(&self) -> Option<PathBuf> {
        let raw = self.output.as_ref()?;
        let fixed = reext(raw, self.kind.extension());
        let bare = fixed.parent().is_none_or(|p| p.as_os_str().is_empty());
        if let (true, Some(name), Some(dir)) =
            (bare, fixed.file_name(), self.inputs.first().and_then(|p| p.parent()))
        {
            return Some(dir.join(name));
        }
        Some(fixed)
    }

    fn ready(&self) -> bool {
        if self.busy() {
            return false;
        }
        match self.mode {
            Mode::Compress => !self.inputs.is_empty() && self.output.is_some(),
            Mode::Extract => {
                self.archive.is_some() && self.archive_format.is_some() && self.dest.is_some()
            }
            Mode::Browse => false,
        }
    }

    fn start_job(&mut self) {
        if !self.ready() {
            return;
        }
        // Lock in a well-formed output name before we hand it to the engine, so
        // a stripped or mistyped extension can't yield a name-less archive.
        if self.mode == Mode::Compress && let Some(fixed) = self.finalized_output() {
            self.output = Some(fixed);
        }
        let progress = Arc::new(Progress::new());
        let outcome: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));

        let p = progress.clone();
        let out = outcome.clone();

        match self.mode {
            Mode::Compress => {
                let inputs = self.inputs.clone();
                let dest = self.output.clone().unwrap();
                let format = self.kind.format();
                let level = self.kind.level_range().map(|_| self.level);
                let opts = CodecOptions::new(level, self.threads.max(0) as u32);
                let label = self.kind.to_string();
                thread::spawn(move || {
                    let start = Instant::now();
                    let result = archive_engine::compress_with_progress(
                        &inputs, &dest, format, &opts, &p,
                    );
                    let msg = match result {
                        Ok(report) => Ok(format!(
                            "Folded {} → {}  ·  {:.0}% saved  ·  {label}  ·  {:.2}s",
                            fmt_bytes(report.uncompressed),
                            fmt_bytes(report.compressed),
                            (1.0 - report.ratio()) * 100.0,
                            start.elapsed().as_secs_f64(),
                        )),
                        Err(e) => Err(e.to_string()),
                    };
                    *out.lock().unwrap() = Some(msg);
                });
            }
            Mode::Extract => {
                let src = self.archive.clone().unwrap();
                let dest = self.dest.clone().unwrap();
                let format = self.archive_format.unwrap();
                thread::spawn(move || {
                    let start = Instant::now();
                    let result =
                        archive_engine::decompress_with_progress(&src, &dest, format, &p);
                    let msg = match result {
                        Ok(()) => Ok(format!(
                            "Unfolded to {}  ·  {:.2}s",
                            dest.display(),
                            start.elapsed().as_secs_f64(),
                        )),
                        Err(e) => Err(e.to_string()),
                    };
                    *out.lock().unwrap() = Some(msg);
                });
            }
            Mode::Browse => return,
        }

        self.job = Some(Job { progress, outcome, started: Instant::now() });
        self.status = Status::Running;
        self.fraction = 0.0;
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        self.fraction = job.progress.fraction();

        let finished = job.outcome.lock().unwrap().take();
        if let Some(result) = finished {
            match result {
                Ok(msg) => {
                    self.fraction = 1.0;
                    self.status = Status::Done(msg);
                }
                Err(msg) => self.status = Status::Failed(msg),
            }
            self.job = None;
        }
    }
}

// --- View ------------------------------------------------------------------

impl Abyss {
    fn view(&self) -> Element<'_, Message> {
        let mut content = column![self.header()].spacing(16).height(Length::Fill);
        if let Some(update) = &self.update {
            content = content.push(self.update_banner(update));
        }
        content = content.push(self.body()).push(self.status_band());

        container(content)
            .style(theme::root)
            .padding(18)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update_banner(&self, update: &UpdateInfo) -> Element<'_, Message> {
        container(
            row![
                text(format!(
                    "★  A new depth has surfaced — v{} awaits.",
                    update.version
                ))
                .size(13)
                .color(theme::TEXT)
                .width(Length::Fill),
                button(text("Update").size(13))
                    .style(theme::primary)
                    .padding([7, 18])
                    .on_press(Message::OpenRelease),
                button(text("Later").size(13))
                    .style(theme::ghost)
                    .padding([7, 14])
                    .on_press(Message::DismissUpdate),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .style(theme::update_banner)
        .padding([10, 16])
        .into()
    }

    fn header(&self) -> Element<'_, Message> {
        let wordmark = column![
            row![
                text("◆").size(26).color(theme::VIOLET),
                text("ABYSSC").size(26).color(theme::CYAN),
                container(
                    text(concat!("v", env!("CARGO_PKG_VERSION"))).size(11).color(theme::CYAN)
                )
                .style(theme::chip)
                .padding([2, 9]),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            text("compression from the depths").size(12).color(theme::MUTED),
        ]
        .spacing(3);

        let tabs = container(
            row![
                self.tab("Compress", Mode::Compress),
                self.tab("Extract", Mode::Extract),
                self.tab("Commander", Mode::Browse),
            ]
            .spacing(4),
        )
        .style(theme::card)
        .padding(4);

        container(
            row![wordmark, Space::with_width(Length::Fill), tabs]
                .align_y(Alignment::Center),
        )
        .style(theme::header)
        .padding([6, 4])
        .into()
    }

    fn tab(&self, label: &'static str, mode: Mode) -> Element<'_, Message> {
        let active = self.mode == mode;
        let style = if active { theme::tab_active } else { theme::tab_inactive };
        button(text(label).size(14))
            .style(style)
            .padding([8, 22])
            .on_press(Message::ModeSelected(mode))
            .into()
    }

    fn body(&self) -> Element<'_, Message> {
        let inner = match self.mode {
            Mode::Compress => self.compress_view(),
            Mode::Extract => self.extract_view(),
            Mode::Browse => self.browse_view(),
        };
        container(inner)
            .style(theme::panel)
            .padding(18)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    // --- Compress ----------------------------------------------------------

    fn compress_view(&self) -> Element<'_, Message> {
        let add_bar = row![
            text("Sources").size(15).color(theme::TEXT),
            Space::with_width(Length::Fill),
            button(text("+ Add files").size(13)).style(theme::ghost).padding([7, 14])
                .on_press_maybe((!self.busy()).then_some(Message::AddFiles)),
            button(text("+ Add folder").size(13)).style(theme::ghost).padding([7, 14])
                .on_press_maybe((!self.busy()).then_some(Message::AddFolder)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let list: Element<_> = if self.inputs.is_empty() {
            container(
                column![
                    text("Drag files & folders into the Abyss").size(15).color(theme::TEXT),
                    text("…or use the buttons above.").size(12).color(theme::MUTED),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .style(theme::dropzone)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
        } else {
            let mut items = column![].spacing(6);
            for (i, path) in self.inputs.iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                let kind = if path.is_dir() { "dir" } else { "file" };
                items = items.push(
                    container(
                        row![
                            text(kind).size(11).color(theme::MUTED).width(Length::Fixed(34.0)),
                            text(name).size(13).color(theme::TEXT).width(Length::Fill),
                            button(text("×").size(15)).style(theme::danger_ghost).padding([2, 9])
                                .on_press_maybe((!self.busy()).then_some(Message::RemoveInput(i))),
                        ]
                        .align_y(Alignment::Center)
                        .spacing(8),
                    )
                    .style(theme::row_item)
                    .padding([6, 10]),
                );
            }

            let footer = row![
                text(format!(
                    "{} item(s) · {}",
                    self.inputs.len(),
                    fmt_bytes(total_input_size(&self.inputs))
                ))
                .size(12)
                .color(theme::MUTED),
                Space::with_width(Length::Fill),
                button(text("Clear all").size(12)).style(theme::danger_ghost).padding([4, 10])
                    .on_press_maybe((!self.busy()).then_some(Message::ClearInputs)),
            ]
            .align_y(Alignment::Center);

            column![scrollable(items).height(Length::Fill), footer].spacing(10).into()
        };

        column![add_bar, list, self.settings_row(), self.output_row()]
            .spacing(16)
            .height(Length::Fill)
            .into()
    }

    fn settings_row(&self) -> Element<'_, Message> {
        let chooser = column![
            text("Format").size(12).color(theme::MUTED),
            pick_list(&ArchiveKind::ALL[..], Some(self.kind), Message::KindSelected)
                .style(theme::picklist)
                .padding([8, 12])
                .width(Length::Fill),
            text(self.kind.tagline()).size(11).color(theme::MUTED),
        ]
        .spacing(6)
        .width(Length::FillPortion(2));

        let level: Element<_> = match self.kind.level_range() {
            Some((min, max, _)) => column![
                row![
                    text("Effort").size(12).color(theme::MUTED),
                    Space::with_width(Length::Fill),
                    text(format!("{}", self.level)).size(12).color(theme::CYAN),
                ],
                slider(min..=max, self.level, Message::LevelChanged),
                text("higher = smaller, slower").size(11).color(theme::MUTED),
            ]
            .spacing(6)
            .width(Length::FillPortion(2))
            .into(),
            None => column![
                text("Effort").size(12).color(theme::MUTED),
                container(text("fixed — one speed: fast").size(12).color(theme::MUTED))
                    .padding([8, 0]),
            ]
            .spacing(6)
            .width(Length::FillPortion(2))
            .into(),
        };

        let threads: Element<_> = if self.kind.uses_threads() {
            let max = available_cores();
            let shown = if self.threads == 0 {
                format!("all ({max})")
            } else {
                self.threads.to_string()
            };
            column![
                row![
                    text("Threads").size(12).color(theme::MUTED),
                    Space::with_width(Length::Fill),
                    text(shown).size(12).color(theme::CYAN),
                ],
                slider(0..=max, self.threads, Message::ThreadsChanged),
                text("0 = every core").size(11).color(theme::MUTED),
            ]
            .spacing(6)
            .width(Length::FillPortion(2))
            .into()
        } else {
            Space::with_width(Length::FillPortion(2)).into()
        };

        container(row![chooser, level, threads].spacing(22).align_y(Alignment::Start))
            .style(theme::card)
            .padding(16)
            .into()
    }

    fn output_row(&self) -> Element<'_, Message> {
        let value = self
            .output
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        row![
            text("Orb →").size(13).color(theme::MUTED),
            text_input("destination archive…", &value)
                .on_input(Message::OutputEdited)
                .style(theme::field)
                .padding([9, 12])
                .width(Length::Fill),
            button(text("Browse").size(13)).style(theme::ghost).padding([9, 16])
                .on_press_maybe((!self.busy()).then_some(Message::BrowseOutput)),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    }

    // --- Extract -----------------------------------------------------------

    fn extract_view(&self) -> Element<'_, Message> {
        let open_bar = row![
            text("Archive").size(15).color(theme::TEXT),
            Space::with_width(Length::Fill),
            self.archive
                .as_ref()
                .map(|p| {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    text(name).size(13).color(theme::TEXT)
                })
                .unwrap_or_else(|| text("nothing opened").size(13).color(theme::MUTED)),
            self.archive_format
                .map(|f| {
                    container(text(f.label()).size(12))
                        .style(theme::chip)
                        .padding([3, 12])
                })
                .map(Element::from)
                .unwrap_or_else(|| Space::with_width(Length::Fixed(0.0)).into()),
            button(text("Open archive").size(13)).style(theme::ghost).padding([7, 14])
                .on_press_maybe((!self.busy()).then_some(Message::OpenArchive)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let contents: Element<_> = match &self.listing {
            Some(listing) if listing.single_stream => container(
                column![
                    text("Single stream").size(14).color(theme::TEXT),
                    text(
                        listing
                            .entries
                            .first()
                            .map(|e| e.name.clone())
                            .unwrap_or_default(),
                    )
                    .size(13)
                    .color(theme::MUTED),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .style(theme::dropzone)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into(),
            Some(listing) => {
                let mut rows = column![row![
                    text("SIZE").size(11).color(theme::MUTED).width(Length::Fixed(110.0)),
                    text("NAME").size(11).color(theme::MUTED),
                ]
                .spacing(8)]
                .spacing(4);
                for entry in &listing.entries {
                    let size = if entry.is_dir {
                        "<dir>".to_string()
                    } else {
                        fmt_bytes(entry.size.unwrap_or(0))
                    };
                    let name_color = if entry.is_dir { theme::CYAN } else { theme::TEXT };
                    rows = rows.push(
                        row![
                            text(size).size(12).color(theme::MUTED).width(Length::Fixed(110.0)),
                            text(entry.name.clone()).size(12).color(name_color),
                        ]
                        .spacing(8),
                    );
                }
                container(scrollable(rows).height(Length::Fill))
                    .style(theme::card)
                    .padding(14)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            None => container(
                text("Open an archive to peer inside it.").size(14).color(theme::MUTED),
            )
            .style(theme::dropzone)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into(),
        };

        let dest_value = self
            .dest
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let dest_row = row![
            text("Unfold →").size(13).color(theme::MUTED),
            container(text(dest_value).size(13).color(theme::TEXT))
                .style(theme::row_item)
                .padding([9, 12])
                .width(Length::Fill),
            button(text("Choose folder").size(13)).style(theme::ghost).padding([9, 16])
                .on_press_maybe((!self.busy()).then_some(Message::ChooseDest)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        column![open_bar, contents, dest_row].spacing(16).height(Length::Fill).into()
    }

    // --- Browse / Commander ------------------------------------------------

    fn browse_view(&self) -> Element<'_, Message> {
        let up = button(text("↑ Up").size(13))
            .style(theme::ghost)
            .padding([7, 14])
            .on_press_maybe(self.location.can_up().then_some(Message::BrowseUp));
        let home = button(text("⌂").size(15)).style(theme::ghost).padding([7, 12])
            .on_press(Message::BrowseHome);
        let refresh = button(text("↻").size(15)).style(theme::ghost).padding([7, 12])
            .on_press(Message::BrowseRefresh);

        let loc_bar = container(text(self.location.label()).size(13).color(theme::TEXT))
            .style(theme::row_item)
            .padding([8, 14])
            .width(Length::Fill);

        // Context action depends on where we stand.
        let action: Element<_> = if self.location.in_archive() {
            button(text("Extract archive ↓").size(13))
                .style(theme::primary)
                .padding([8, 16])
                .on_press(Message::BrowseExtractHere)
                .into()
        } else if self.selected_fs_path().is_some() {
            button(text("→ Add to Compress").size(13))
                .style(theme::ghost)
                .padding([8, 16])
                .on_press(Message::BrowseToCompress)
                .into()
        } else {
            Space::with_width(Length::Fixed(0.0)).into()
        };

        let toolbar = row![up, home, refresh, loc_bar, action]
            .spacing(8)
            .align_y(Alignment::Center);

        let mut list = column![].spacing(2);
        for (i, row) in self.rows.iter().enumerate() {
            list = list.push(self.browse_line(i, row));
        }

        let body: Element<_> = if let Some(err) = &self.browse_error {
            column![
                container(scrollable(list).height(Length::Fill))
                    .style(theme::card)
                    .padding(10)
                    .height(Length::Fill),
                text(format!("× {err}")).size(12).color(theme::RED),
            ]
            .spacing(8)
            .into()
        } else {
            container(scrollable(list).height(Length::Fill))
                .style(theme::card)
                .padding(10)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        column![toolbar, body].spacing(14).height(Length::Fill).into()
    }

    fn browse_line(&self, i: usize, row: &BrowseRow) -> Element<'_, Message> {
        let (glyph, glyph_color) = match row.kind {
            RowKind::Parent => ("↑", theme::MUTED),
            RowKind::Drive => ("▣", theme::CYAN),
            RowKind::Dir => ("▶", theme::CYAN),
            RowKind::Archive => ("◆", theme::VIOLET),
            RowKind::File => ("•", theme::MUTED),
        };
        let name_color = match row.kind {
            RowKind::Archive => theme::VIOLET,
            RowKind::Dir | RowKind::Drive => theme::CYAN,
            RowKind::Parent => theme::MUTED,
            RowKind::File => theme::TEXT,
        };
        let size = match row.kind {
            RowKind::File | RowKind::Archive => fmt_bytes(row.size.unwrap_or(0)),
            _ => String::new(),
        };
        let style =
            if self.selected == Some(i) { theme::browse_row_selected } else { theme::browse_row };

        button(
            row![
                text(glyph).size(14).color(glyph_color).width(Length::Fixed(26.0)),
                text(row.name.clone()).size(13).color(name_color).width(Length::Fill),
                text(size).size(12).color(theme::MUTED),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .style(style)
        .padding([6, 10])
        .width(Length::Fill)
        .on_press(Message::BrowseActivate(i))
        .into()
    }

    fn browse_status(&self) -> Element<'_, Message> {
        let (msg, color) = if let Some(name) = &self.opening {
            (format!("◓  Drawing “{name}” from the depths — it will open shortly…"), theme::CYAN)
        } else if let Some(err) = &self.browse_error {
            (err.clone(), theme::RED)
        } else if let Some(path) = self.selected_fs_path() {
            (path.display().to_string(), theme::TEXT)
        } else {
            let count = self.rows.iter().filter(|r| r.kind != RowKind::Parent).count();
            let hint = if self.location.in_archive() {
                "peering inside — nothing is unpacked to disk"
            } else {
                "double-step a folder or archive to enter"
            };
            (format!("{count} item(s)  ·  {hint}"), theme::MUTED)
        };
        container(text(msg).size(13).color(color).width(Length::Fill))
            .style(theme::status_bar)
            .padding(16)
            .into()
    }

    // --- Status band -------------------------------------------------------

    fn status_band(&self) -> Element<'_, Message> {
        if self.mode == Mode::Browse {
            return self.browse_status();
        }

        // "Sealing": every input byte is in, but the codec is still finalizing
        // (zstd at high effort can grind here for a long time). Without this the
        // bar would just sit at 100% and look hung.
        let sealing = self.busy() && self.fraction >= 1.0;
        let elapsed = self.job.as_ref().map(|j| j.started.elapsed()).unwrap_or_default();

        let (msg, msg_color): (String, _) = match &self.status {
            Status::Idle => (self.idle_hint(), theme::MUTED),
            Status::Running if sealing => {
                const SPIN: [&str; 4] = ["◐", "◓", "◑", "◒"];
                let frame = SPIN[(elapsed.as_millis() / 120 % SPIN.len() as u128) as usize];
                let phase = match self.mode {
                    Mode::Compress => "Sealing the orb — crushing the final depths",
                    Mode::Extract => "Drawing up the last of it",
                    Mode::Browse => "",
                };
                (format!("{frame}  {phase}…  ·  {:.0}s", elapsed.as_secs_f64()), theme::CYAN)
            }
            Status::Running => (
                format!(
                    "{} from the depths… {:.0}%",
                    match self.mode {
                        Mode::Compress => "Folding",
                        Mode::Extract => "Unfolding",
                        Mode::Browse => "",
                    },
                    self.fraction * 100.0
                ),
                theme::CYAN,
            ),
            Status::Done(m) => (m.clone(), theme::GREEN),
            Status::Failed(m) => (format!("× {m}"), theme::RED),
        };

        let action_label = match self.mode {
            Mode::Compress => "Compress",
            Mode::Extract => "Extract",
            Mode::Browse => "",
        };
        let action = button(text(action_label).size(15))
            .style(theme::primary)
            .padding([11, 30])
            .on_press_maybe(self.ready().then_some(Message::Start));

        let mut band = column![].spacing(10);
        if self.busy() || matches!(self.status, Status::Done(_)) {
            // While sealing, sweep the bar back and forth so it reads as "busy"
            // rather than a frozen 100%.
            let bar_value = if sealing {
                let t = (elapsed.as_millis() % 1400) as f32 / 1400.0;
                1.0 - (2.0 * t - 1.0).abs()
            } else {
                self.fraction
            };
            band = band.push(
                progress_bar(0.0..=1.0, bar_value)
                    .height(Length::Fixed(8.0))
                    .style(theme::progress),
            );
        }
        band = band.push(
            row![
                text(msg).size(13).color(msg_color).width(Length::Fill),
                action,
            ]
            .align_y(Alignment::Center)
            .spacing(12),
        );

        container(band).style(theme::status_bar).padding(16).into()
    }

    fn idle_hint(&self) -> String {
        match self.mode {
            Mode::Compress if self.inputs.is_empty() => {
                "Add sources to begin.".to_string()
            }
            Mode::Compress if self.output.is_none() => {
                "Name the orb to compress into.".to_string()
            }
            Mode::Compress => "Ready to fold.".to_string(),
            Mode::Extract if self.archive_format.is_none() => {
                "Open an archive to begin.".to_string()
            }
            Mode::Extract if self.dest.is_none() => {
                "Choose where to unfold it.".to_string()
            }
            Mode::Extract => "Ready to unfold.".to_string(),
            Mode::Browse => String::new(),
        }
    }
}

// --- Free helpers ----------------------------------------------------------

/// Replace a path's recognized archive extension with `new_ext`.
fn reext(path: &PathBuf, new_ext: &str) -> PathBuf {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    const KNOWN: &[&str] = &[
        ".tar.zst", ".tar.lz4", ".tar.xz", ".tar.gz", ".tar.bz2", ".tar.br", ".tgz", ".tzst",
        ".txz", ".tbz2", ".tbz", ".zip", ".tar", ".zst", ".gz", ".lz4", ".xz", ".bz2", ".br",
    ];
    let stem = KNOWN
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map(|ext| name[..name.len() - ext.len()].to_string())
        .unwrap_or(name);
    let new_name = format!("{stem}{new_ext}");
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(new_name),
        _ => PathBuf::from(new_name),
    }
}

fn total_input_size(inputs: &[PathBuf]) -> u64 {
    inputs.iter().map(|p| dir_size(p)).sum()
}

fn dir_size(path: &std::path::Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path)
            .map(|entries| entries.flatten().map(|e| dir_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(meta) => meta.len(),
        Err(_) => 0,
    }
}

fn available_cores() -> i32 {
    std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(1)
}

/// Draw a single member out of an archive into a private temp folder and hand it
/// to the OS to open with the user's default app — no full extraction. Runs on a
/// blocking worker thread.
fn open_archive_member(src: &Path, format: Format, member: &str, name: &str) -> Result<(), String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dest = std::env::temp_dir().join("AbyssC").join(stamp.to_string()).join(name);
    archive_engine::extract_member(src, format, member, &dest).map_err(|e| e.to_string())?;
    open_path(&dest);
    Ok(())
}

/// Open a filesystem path with the OS default handler.
fn open_path(path: &Path) {
    #[cfg(windows)]
    {
        // `start` is a cmd builtin; the empty "" is its (ignored) window title.
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

/// Open a URL in the user's default browser.
fn open_url(url: &str) {
    #[cfg(windows)]
    {
        // `start` is a cmd builtin; the empty "" is its (ignored) window title.
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn fmt_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}
