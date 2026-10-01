// COSMIC panel applet - application model, update loop, and view.
//
// Panel is the headline, popup is the full board. The panel button shows one
// repo as `soulless main ↑2`; clicking it opens a popup that lists every repo
// currently open, each with its branch, ahead/behind, and uncommitted-change
// count. The data comes from `scan::snapshot`, re-read every REFRESH_INTERVAL
// on a background thread so a slow status walk never stalls the panel.

use std::time::Duration;

use cosmic::app::{Core, Task};
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::{Alignment, Length, Rectangle, Subscription, window};
use cosmic::widget::rectangle_tracker::{
    RectangleTracker, RectangleUpdate, rectangle_tracker_subscription,
};
use cosmic::widget::{Column, button, column, container, row, space, text};
use cosmic::{Element, surface};

use crate::fl;
use crate::repo::RepoStatus;
use crate::scan;

/// How often the board is re-read. Git state changes constantly while
/// working (every save dirties a file, every commit moves the arrows), so
/// this is short; the read itself is a few milliseconds per repo.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// The popup's fixed width. 360 is what libcosmic gives applet popups by
/// default; the height is left to autosize, since the board is a list that
/// grows and shrinks with however many repos are open.
const POPUP_WIDTH: f32 = 360.0;

/// The application model stores app-specific state used to describe its
/// interface and drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: Core,
    /// The popup id, while the popup is open.
    popup: Option<window::Id>,
    /// The board as of the last refresh. `repos[0]` is the headline the panel
    /// shows; the popup lists them all. Empty = no repo open anywhere.
    repos: Vec<RepoStatus>,
    /// True while a refresh is running on the background thread. Ticks that
    /// land during one are dropped rather than queued, so a slow read never
    /// piles up a backlog of reads behind it.
    refreshing: bool,
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
    Refreshed(Vec<RepoStatus>),
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "io.github.hmrdsmoke.Branchkeeper";

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
            repos: Vec::new(),
            refreshing: false,
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

    /// The popup: the full board, one row per open repo.
    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let spacing = cosmic::theme::spacing();

        let board: Element<'_, Message> = if self.repos.is_empty() {
            text(fl!("no-open-repos")).size(14).into()
        } else {
            let rows = self
                .repos
                .iter()
                .map(|repo| self.repo_row(repo, spacing))
                .collect::<Vec<Element<'_, Message>>>();
            Column::with_children(rows)
                .spacing(spacing.space_s)
                .width(Length::Fill)
                .into()
        };

        let content = container(board)
            .width(Length::Fixed(POPUP_WIDTH))
            .padding([spacing.space_s, spacing.space_m]);
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
            Message::Refreshed(repos) => {
                self.refreshing = false;
                self.repos = repos;
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

        cosmic::task::future(async {
            // libgit2 does real file I/O - the index, the refs, a walk of the
            // working tree for status - so it runs on tokio's blocking pool,
            // not on the executor threads that drive the UI. If the thread
            // panics (it shouldn't; every git error is a Result), treat it as
            // an empty board rather than taking the applet down with it.
            let repos = tokio::task::spawn_blocking(scan::snapshot)
                .await
                .unwrap_or_default();
            Message::Refreshed(repos)
        })
    }

    /// The panel text: the headline repo, or a placeholder when none is open.
    fn panel_label(&self) -> String {
        match self.repos.first() {
            Some(repo) => repo.headline(),
            None => fl!("no-repo"),
        }
    }

    /// One row of the board. Top line reads like the panel - `name branch` on
    /// the left, `↑N ↓N` on the right - then a smaller line with the change
    /// count.
    fn repo_row<'a>(&'a self, repo: &'a RepoStatus, spacing: Spacing) -> Element<'a, Message> {
        let title = format!("{} {}", repo.name, repo.branch);
        let top = row::with_capacity(2)
            .push(text(title).size(14).width(Length::Fill))
            .push(text(sync_detail(repo)).size(14))
            .align_y(Alignment::Center);

        column::with_capacity(2)
            .push(top)
            .push(text(changes_detail(repo)).size(12))
            .spacing(spacing.space_xxxs)
            .width(Length::Fill)
            .into()
    }

    /// Vertical panels get the headline one word per line, the way Void
    /// Watcher stacks its clock, so `soulless main ↑2` reads top to bottom.
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
