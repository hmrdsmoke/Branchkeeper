// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// branchkeeper/src/app.rs
// src/app.rs

// COSMIC panel applet - application model, update loop, and view.
//
// Panel is the headline, popup is the full board. The panel button shows one
// repo as `soulless main ↑2 ↓0`; clicking it opens a popup with two sections:
// the repos that have a shell in them (most recently touched first), then
// every other project under the user's project folders. Every row is a
// button that opens a terminal in that repo. A Settings screen at the bottom
// of the popup manages the project folders, with a native folder picker.
// The data comes from `scan::snapshot`, re-read every REFRESH_INTERVAL on a
// background thread so a slow status walk never stalls the panel.

use std::path::PathBuf;
use std::time::Duration;

use cosmic::app::{Core, Task};
use cosmic::applet::{menu_button, padded_control};
use cosmic::cosmic_theme::Spacing;
use cosmic::dialog::file_chooser;
use cosmic::iced::{Alignment, Length, Rectangle, Subscription, window};
use cosmic::widget::rectangle_tracker::{
    RectangleTracker, RectangleUpdate, rectangle_tracker_subscription,
};
use cosmic::widget::{
    Column, button, column, container, divider, icon, row, scrollable, space, text,
};
use cosmic::{Element, surface};

use crate::fetch;
use crate::fl;
use crate::launch;
use crate::repo::RepoStatus;
use crate::scan::{self, Board};
use crate::settings::{self, Settings};

/// How often the board is re-read. Git state changes constantly while
/// working (every save dirties a file, every commit moves the arrows), so
/// this is short; the read itself is a few milliseconds per repo.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// The popup's fixed width. 360 is what libcosmic gives applet popups by
/// default; the height is left to autosize, since the board is a list that
/// grows and shrinks with however many repos there are - up to a cap.
const POPUP_WIDTH: f32 = 360.0;

/// How many repo rows the board shows before it starts scrolling. Keeps the
/// popup manageable for someone with a folder full of repos.
const VISIBLE_ROWS: f32 = 5.0;

/// Which screen the popup is showing. The popup is a single surface that
/// swaps between them, because an applet can't put a second real window on
/// screen - the panel is a nested compositor and would swallow any toplevel
/// it opened. (The folder picker is the exception: that's the desktop
/// portal's window, not ours.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Board,
    Settings,
}

/// The application model stores app-specific state used to describe its
/// interface and drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: Core,
    /// The popup id, while the popup is open.
    popup: Option<window::Id>,
    /// Which screen the popup shows.
    screen: Screen,
    /// Branchkeeper's own preferences (the project folders), loaded at start
    /// and saved on every change by the settings screen.
    settings: Settings,
    /// The board as of the last refresh: open repos (headline first) and the
    /// rest of the projects. See `scan::Board`.
    board: Board,
    /// True while a refresh is running on the background thread. Ticks that
    /// land during one are dropped rather than queued, so a slow read never
    /// piles up a backlog of reads behind it.
    refreshing: bool,
    /// True while a "Fetch all" is running on the background thread. The
    /// button reads this to show "Fetching…" and ignore further presses.
    fetching: bool,
    /// Handle to the panel's rectangle tracker, delivered once at startup.
    rectangle_tracker: Option<RectangleTracker<u32>>,
    /// The panel button's true on-screen rectangle, reported by the tracker.
    rectangle: Rectangle,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    /// The panel button was pressed - open or close the popup.
    TogglePopup,
    /// An update from the panel's rectangle tracker.
    Rectangle(RectangleUpdate<u32>),
    PopupClosed(window::Id),
    /// The refresh timer fired: start a background read of the board.
    Tick,
    /// A background read finished; this is the new board.
    Refreshed(Board),
    /// A board row was clicked: open a terminal in that repo's directory.
    Launch(PathBuf),
    /// Open the settings screen.
    OpenSettings,
    /// Leave the settings screen back to the board.
    CloseSettings,
    /// "Add folder…" was clicked: show the folder picker.
    AddFolder,
    /// The folder picker came back - with a folder, or `None` if cancelled.
    FolderPicked(Option<PathBuf>),
    /// Remove the project folder at this position in the settings list.
    RemoveFolder(usize),
    /// "Fetch all" was clicked: fetch every repo on the board in the background.
    FetchAll,
    /// A fetch-all finished; refresh the board so the ↓ counts reflect it.
    Fetched(crate::fetch::FetchReport),
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = settings::CONFIG_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Message>) {
        let mut app = AppModel {
            core,
            popup: None,
            screen: Screen::Board,
            settings: Settings::load(),
            board: Board::default(),
            refreshing: false,
            fetching: false,
            rectangle_tracker: None,
            rectangle: Rectangle::default(),
        };

        // Read the board once right away rather than showing "no repo" until
        // the first tick comes around.
        let first_read = app.refresh();
        (app, first_read)
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Message> {
        let horizontal = self.core.applet.is_horizontal();

        let label: Element<'_, Message> = if horizontal {
            let fill_height = (self.core.applet.suggested_size(true).1
                + 2 * self.core.applet.suggested_padding(true).1)
                as f32;
            row(vec![
                self.core.applet.text(self.panel_label()).into(),
                container(space::vertical().height(Length::Fixed(fill_height))).into(),
            ])
            .align_y(Alignment::Center)
            .into()
        } else {
            self.stacked_label()
        };

        let (along, _across) = self.core.applet.suggested_padding(true);
        let padding = if horizontal { [0, along] } else { [along, 0] };

        let button = button::custom(label)
            .class(cosmic::theme::Button::AppletIcon)
            .padding(padding)
            .on_press_down(Message::TogglePopup);

        let content: Element<'_, Message> = match self.rectangle_tracker.as_ref() {
            Some(tracker) => tracker.container(0, button).ignore_bounds(true).into(),
            None => button.into(),
        };

        self.core.applet.autosize_window(content).into()
    }

    /// The popup: the board, or the settings screen.
    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let screen: Element<'_, Message> = match self.screen {
            Screen::Board => self.board_screen(),
            Screen::Settings => self.settings_screen(),
        };

        let content = container(screen).width(Length::Fixed(POPUP_WIDTH));
        self.core.applet.popup_container(content).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            cosmic::iced::time::every(REFRESH_INTERVAL).map(|_| Message::Tick),
            rectangle_tracker_subscription(0).map(|update| Message::Rectangle(update.1)),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TogglePopup => {
                if let Some(id) = self.popup.take() {
                    return surface::surface_task(surface::action::destroy_popup(id));
                }

                // Always open on the board; Settings is a place you go, not a
                // place you come back to.
                self.screen = Screen::Board;

                return surface::surface_task(surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut Self| {
                        let parent = app
                            .core
                            .main_window_id()
                            .expect("applet main window exists before its popup opens");
                        let id = window::Id::unique();
                        app.popup = Some(id);

                        let mut settings = app
                            .core
                            .applet
                            .get_popup_settings(parent, id, None, None, None);

                        // Anchor the popup to the button's real rectangle (from
                        // the tracker) rather than the panel's guess, so it
                        // opens under the button wherever the panel put us.
                        let Rectangle {
                            x,
                            y,
                            width,
                            height,
                        } = app.rectangle;
                        settings.positioner.anchor_rect = Rectangle::<i32> {
                            x: x.max(1.0) as i32,
                            y: y.max(1.0) as i32,
                            width: width.max(1.0) as i32,
                            height: height.max(1.0) as i32,
                        };

                        // Width pinned, height free: libcosmic's defaults
                        // already do that (360 wide, autosized height), and
                        // the board is a list, so leave them be.
                        settings
                    },
                    None,
                ));
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::Rectangle(update) => match update {
                RectangleUpdate::Rectangle((_, rect)) => {
                    self.rectangle = rect;
                }
                RectangleUpdate::Init(tracker) => {
                    self.rectangle_tracker = Some(tracker);
                }
            },
            Message::Tick => return self.refresh(),
            Message::Refreshed(board) => {
                self.refreshing = false;
                self.board = board;
            }
            Message::Launch(dir) => {
                launch::terminal_in(&dir);
                // Picking a row is a choice made; close the popup like any
                // menu would. The new shell shows up in "Open" on the next
                // refresh, and that repo becomes the headline.
                if let Some(id) = self.popup.take() {
                    return surface::surface_task(surface::action::destroy_popup(id));
                }
            }
            Message::OpenSettings => {
                self.screen = Screen::Settings;
            }
            Message::CloseSettings => {
                self.screen = Screen::Board;
            }
            Message::AddFolder => {
                // The picker is the desktop portal's own window (on COSMIC,
                // cosmic-files' dialog), so it's a real toplevel with focus.
                // Taking focus dismisses our popup - that's expected; the
                // choice lands in FolderPicked whether the popup survived or
                // not, and the next open shows the new folder's repos.
                return cosmic::task::future(async {
                    let dialog = file_chooser::open::Dialog::new().title(fl!("pick-folder"));
                    match dialog.open_folder().await {
                        Ok(response) => Message::FolderPicked(response.url().to_file_path().ok()),
                        Err(file_chooser::Error::Cancelled) => Message::FolderPicked(None),
                        Err(why) => {
                            eprintln!("branchkeeper: folder picker failed: {why}");
                            Message::FolderPicked(None)
                        }
                    }
                });
            }
            Message::FolderPicked(Some(dir)) => {
                self.settings.add_root(&dir);
                // Re-read right away so the new folder's repos are on the
                // board the moment the popup opens again.
                return self.refresh();
            }
            Message::FolderPicked(None) => {}
            Message::RemoveFolder(index) => {
                self.settings.remove_root(index);
                return self.refresh();
            }
            Message::FetchAll => {
                // Ignore a second press while one's already running.
                if self.fetching {
                    return Task::none();
                }
                self.fetching = true;

                // Fetch exactly the repos currently on the board - open ones
                // and projects both - by their working-tree paths.
                let dirs: Vec<PathBuf> = self
                    .board
                    .open
                    .iter()
                    .chain(self.board.projects.iter())
                    .map(|repo| repo.workdir.clone())
                    .collect();

                return cosmic::task::future(async move {
                    // Network + libgit2 I/O, so off the UI thread like the
                    // status read. A panic becomes an empty report rather than
                    // taking the applet down.
                    let report = tokio::task::spawn_blocking(move || fetch::fetch_all(&dirs))
                        .await
                        .unwrap_or_default();
                    Message::Fetched(report)
                });
            }
            Message::Fetched(report) => {
                self.fetching = false;
                eprintln!(
                    "branchkeeper: fetched {} of {} repos",
                    report.succeeded, report.attempted
                );
                // Re-read so the freshly updated remote refs show in the ↓.
                return self.refresh();
            }
        }

        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    /// Kick off a board read on a background thread. Returns a no-op task if
    /// one is already in flight (see `refreshing`).
    fn refresh(&mut self) -> Task<Message> {
        if self.refreshing {
            return Task::none();
        }
        self.refreshing = true;

        // The thread gets its own copy of the folder list; settings can
        // change while it runs and the next refresh picks that up.
        let roots = self.settings.project_roots.clone();

        cosmic::task::future(async move {
            // libgit2 does real file I/O - the index, the refs, a walk of the
            // working tree for status - so it runs on tokio's blocking pool,
            // not on the executor threads that drive the UI. If the thread
            // panics (it shouldn't; every git error is a Result), treat it as
            // an empty board rather than taking the applet down with it.
            let board = tokio::task::spawn_blocking(move || scan::snapshot(&roots))
                .await
                .unwrap_or_default();
            Message::Refreshed(board)
        })
    }

    /// The panel text: the headline repo, or a placeholder when none is open.
    fn panel_label(&self) -> String {
        match self.board.headline() {
            Some(repo) => repo.headline(),
            None => fl!("no-repo"),
        }
    }

    /// The board: "Open" first - the repos with a shell in them, newest
    /// first - then "Projects", everything else under the project folders.
    /// That list shows about VISIBLE_ROWS rows and scrolls for the rest, so
    /// forty repos don't make a popup taller than the screen. Under it, pinned
    /// so they're always reachable: a divider and the way into Settings.
    /// Each repo row is a button that opens a terminal there.
    fn board_screen(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();

        let mut rows: Vec<Element<'_, Message>> = Vec::new();

        if self.board.is_empty() {
            rows.push(padded_control(text(fl!("no-open-repos")).size(14)).into());
        }

        if !self.board.open.is_empty() {
            rows.push(section_header(fl!("open")));
            rows.extend(
                self.board
                    .open
                    .iter()
                    .map(|repo| self.repo_row(repo, spacing)),
            );
        }

        if !self.board.projects.is_empty() {
            rows.push(section_header(fl!("projects")));
            rows.extend(
                self.board
                    .projects
                    .iter()
                    .map(|repo| self.repo_row(repo, spacing)),
            );
        }

        // The cap is in pixels, worked out from what a row is made of at the
        // current density: a 14pt title line and a 12pt detail line (iced
        // draws text at 1.3x its size), the gap between them, and the menu
        // button's padding above and below. Plus one section header's worth,
        // so "Projects" and five of its rows fit without a scroll.
        let row_height = 14.0 * 1.3
            + 12.0 * 1.3
            + f32::from(spacing.space_xxxs)
            + 2.0 * f32::from(spacing.space_xxs);
        let header_height = 12.0 * 1.3 + 2.0 * f32::from(spacing.space_xxs);
        let list_max_height = VISIBLE_ROWS * row_height + header_height;

        let list = container(scrollable(Column::with_children(rows).width(Length::Fill)))
            .max_height(list_max_height);

        let footer_divider = padded_control(divider::horizontal::default())
            .padding([spacing.space_xxs, spacing.space_m]);

        // "Fetch all" updates every repo's remote refs so the ↓ counts are
        // real. While it runs it reads "Fetching…" and is dead to presses
        // (no on_press), so you can't stack a dozen fetches on one click.
        let fetch_label = if self.fetching {
            fl!("fetching")
        } else {
            fl!("fetch-all")
        };
        let mut fetch_button = menu_button(text(fetch_label).size(14));
        if !self.fetching {
            fetch_button = fetch_button.on_press(Message::FetchAll);
        }

        let settings_button =
            menu_button(text(fl!("settings")).size(14)).on_press(Message::OpenSettings);

        // Rows carry their own horizontal padding (menu_button/padded_control),
        // so the column only pads top and bottom - the stock applets' layout.
        column::with_capacity(4)
            .push(list)
            .push(footer_divider)
            .push(fetch_button)
            .push(settings_button)
            .padding([8, 0])
            .width(Length::Fill)
            .into()
    }

    /// The settings screen: a back button and title, then the project
    /// folders - one row each with a remove button - and "Add folder…".
    fn settings_screen(&self) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();

        let back = button::icon(icon::from_name("go-previous-symbolic").size(16))
            .on_press(Message::CloseSettings);
        let header = padded_control(
            row::with_capacity(2)
                .push(back)
                .push(text(fl!("settings")).size(16))
                .spacing(spacing.space_xs)
                .align_y(Alignment::Center),
        );

        let mut rows: Vec<Element<'_, Message>> = vec![header.into()];
        rows.push(section_header(fl!("project-folders")));

        if self.settings.project_roots.is_empty() {
            rows.push(padded_control(text(fl!("no-folders")).size(14)).into());
        }
        for (index, root) in self.settings.project_roots.iter().enumerate() {
            let remove = button::icon(icon::from_name("edit-delete-symbolic").size(16))
                .on_press(Message::RemoveFolder(index));
            let line = row::with_capacity(2)
                .push(
                    text(settings::display_root(root))
                        .size(14)
                        .width(Length::Fill),
                )
                .push(remove)
                .align_y(Alignment::Center);
            rows.push(padded_control(line).into());
        }

        rows.push(
            menu_button(text(fl!("add-folder")).size(14))
                .on_press(Message::AddFolder)
                .into(),
        );

        Column::with_children(rows)
            .padding([8, 0])
            .width(Length::Fill)
            .into()
    }

    /// One row of the board, as a button that opens a terminal in the repo.
    /// Top line reads like the panel - `name branch` on the left, `↑N ↓N` on
    /// the right - then a smaller line with the change count.
    fn repo_row<'a>(&'a self, repo: &'a RepoStatus, spacing: Spacing) -> Element<'a, Message> {
        let title = format!("{} {}", repo.name, repo.branch);
        let top = row::with_capacity(2)
            .push(text(title).size(14).width(Length::Fill))
            .push(text(sync_detail(repo)).size(14))
            .align_y(Alignment::Center);

        let content = column::with_capacity(2)
            .push(top)
            .push(text(changes_detail(repo)).size(12))
            .spacing(spacing.space_xxxs)
            .width(Length::Fill);

        menu_button(content)
            .on_press(Message::Launch(repo.workdir.clone()))
            .into()
    }

    /// Vertical panels get the headline one word per line, the way Void
    /// Watcher stacks its clock, so `soulless main ↑2 ↓0` reads top to bottom.
    fn stacked_label(&self) -> Element<'_, Message> {
        let label = self.panel_label();
        let lines = label
            .split_whitespace()
            .map(|word| self.core.applet.text(word.to_owned()).into())
            .collect::<Vec<Element<'_, Message>>>();

        let stacked = Column::with_children(lines)
            .align_x(Alignment::Center)
            .spacing(4);

        let fill_width = (self.core.applet.suggested_size(true).0
            + 2 * self.core.applet.suggested_padding(true).1) as f32;

        column(vec![
            stacked.into(),
            space::horizontal().width(Length::Fixed(fill_width)).into(),
        ])
        .align_x(Alignment::Center)
        .into()
    }
}

/// A small section label ("Open", "Projects") with the same side padding as
/// the rows under it.
fn section_header<'a>(label: String) -> Element<'a, Message> {
    padded_control(text(label).size(12)).into()
}

/// The popup's wording for ahead/behind: the same `↑N ↓N` the panel shows,
/// zeros included, or "no upstream" when there's nothing to compare to.
fn sync_detail(repo: &RepoStatus) -> String {
    if repo.tracking.is_some() {
        repo.sync_label()
    } else {
        fl!("no-upstream")
    }
}

/// The popup's wording for the working tree: "clean" or "N changed".
fn changes_detail(repo: &RepoStatus) -> String {
    if repo.is_dirty() {
        fl!("changed", count = repo.changed)
    } else {
        fl!("clean")
    }
}
