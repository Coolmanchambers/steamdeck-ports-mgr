//! Draws a [`Page`] in the guided-screen window (`ui/fork/setup-wizard.slint`): header, title, text, a card,
//! cover tiles, big options and footer buttons. The setup wizard, the Import ports cards, the cloud, save
//! finder, Steam close and old-copies pages all show their pages through [`show_page`]; what a chosen item
//! means is theirs to say (the `A` of the page).
//!
//! **One window.** The first page opens the window; every later page is drawn in the same window, with the
//! new page's handlers swapped in (`HOST`), so moving between pages never closes and reopens a window. When a
//! page's owner closes the dialog to hand over to another screen (`close_current_dialog`), the window is
//! *parked* for a moment ([`park`]) instead of closed: a page shown in that moment takes it back; otherwise it
//! is hidden and the main window gets the focus, as a normal close does. The dialog slot always says what is
//! on screen (a page swap happens inside one callback), so the setup wizard's hand-off watcher never sees a
//! gap between two pages.
//!
//! Only the controller's D-pad, A and B (and the same keys on a keyboard, or the mouse) reach the window:
//! the D-pad moves between options and buttons, A chooses, B goes back one page (`on_back`), and the title
//! bar's X or the window's own close leaves the whole thing (`on_exit`). A page shown again (B, a footer Back
//! button, the wizard coming back after a hand-off) starts with the cursor where it was, when that place still exists.
//!
//! Covers arrive after the page is up: they are fetched and decoded on a background thread
//! (`fork::art::thumbs`) and a timer swaps them in, so a page never waits on the network. Until a cover is
//! there (and when there is none) the port's name is shown on a plain tile.

use super::dialogs::{build_setup_wizard_dialog, close_current_dialog, dialog_is_open, finish_dialog_open, DialogSlot};
use super::state::AppState;
use crate::fork::art::thumbs::{ArtRef, ThumbLoader};
use crate::fork::page::{self, Focus, Page};
use crate::fork::press_echo::{self, Press};
use crate::ui::chrome;
use crate::ui::gamepad_router::GamepadRouter;
use crate::{SetupWizardDialog, WizardCard, WizardFact, WizardOption, WizardTile};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

type Router = Rc<RefCell<GamepadRouter>>;

/// What the page's owner does with each thing the user can do on it.
pub(crate) struct Handlers<A> {
    /// An option or button was chosen.
    pub on_action: Rc<dyn Fn(&Rc<AppState>, &Router, A)>,
    /// B or Escape.
    pub on_back: Rc<dyn Fn(&Rc<AppState>, &Router)>,
    /// The title bar's X, or the window system's own close.
    pub on_exit: Rc<dyn Fn(&Rc<AppState>, &Router)>,
    /// The text box was edited (pages with one): the page as it should read now, or `None` to leave it.
    pub on_input: Option<Rc<dyn Fn(&Rc<AppState>, &Router, &str) -> Option<Page<A>>>>,
}

impl<A> Clone for Handlers<A> {
    fn clone(&self) -> Self {
        Handlers { on_action: self.on_action.clone(), on_back: self.on_back.clone(), on_exit: self.on_exit.clone(), on_input: self.on_input.clone() }
    }
}

const ART_TICK: Duration = Duration::from_millis(150);
/// How long a closed page window waits for the next page before it is really hidden. Long enough for a page
/// that its owner shows from a zero-delay timer after closing (Slint runs timers in deadline order, so a 0 ms
/// timer started after the close fires before this 40 ms one), short enough that nobody sees it linger.
const PARK: Duration = Duration::from_millis(40);
/// Pages remembered for "the cursor comes back to where you were".
const HISTORY_LEN: usize = 24;

/// The covers seen so far: `Some(image)`, or `None` when the cover could not be had (keep the name tile).
struct ArtState {
    cache_dir: PathBuf,
    loader: ThumbLoader,
    images: HashMap<String, Option<slint::Image>>,
}

/// The bookkeeping of the page window, apart from the window itself, so it can be tested without a display.
#[derive(Default)]
struct Host {
    /// The serial of the page window that exists (on screen or parked). Each window built gets a new one; its
    /// callbacks carry it, so a window that is gone can never reach a page.
    window: Option<u64>,
    /// While parked: the token of the park that will hide it.
    parked: Option<u64>,
    /// The page on screen, with the serial of the window that shows it. `None` while the window is parked: a key
    /// that reaches a parked window does nothing.
    page: Option<(u64, Rc<dyn Binding>)>,
    /// Bumped by every page shown and every park: a [`PageHandle`] is live only while it matches.
    generation: u64,
    serials: u64,
}

impl Host {
    /// A new page window was built: it is the one from now on (an older one is forgotten).
    fn new_window(&mut self) -> u64 {
        self.serials += 1;
        self.window = Some(self.serials);
        self.parked = None;
        self.page = None;
        self.serials
    }

    /// A page is now on the window `serial`; its handle's generation.
    fn show(&mut self, serial: u64, binding: Rc<dyn Binding>) -> u64 {
        if self.window == Some(serial) {
            self.parked = None;
        }
        self.page = Some((serial, binding));
        self.generation += 1;
        self.generation
    }

    /// The window `serial` was closed by its page's owner: it waits for the next page. The token its timer brings
    /// back, or `None` when that is not the page window on screen (it is closed normally then).
    fn park(&mut self, serial: u64) -> Option<u64> {
        if self.window != Some(serial) || self.parked.is_some() {
            return None;
        }
        self.generation += 1;
        self.parked = Some(self.generation);
        self.page = None;
        self.parked
    }

    /// A page takes the parked window back: its serial.
    fn retake(&mut self) -> Option<u64> {
        self.parked.take()?;
        self.window
    }

    /// The park's time is up: true when the window is still parked by it (it is hidden and forgotten now).
    fn expire(&mut self, token: u64) -> bool {
        if self.parked != Some(token) {
            return false;
        }
        self.window = None;
        self.parked = None;
        true
    }

    fn bound(&self, serial: u64) -> Option<Rc<dyn Binding>> {
        self.page.as_ref().filter(|(s, _)| *s == serial).map(|(_, b)| b.clone())
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    /// The page window exists and is not parked.
    fn on_screen(&self) -> bool {
        self.window.is_some() && self.parked.is_none()
    }
}

thread_local! {
    static ART: RefCell<Option<ArtState>> = const { RefCell::new(None) };
    static ART_TIMER: slint::Timer = slint::Timer::default();
    static HOST: RefCell<Host> = RefCell::new(Host::default());
    /// The page window itself, with its serial.
    static WIN: RefCell<Option<(u64, SetupWizardDialog)>> = const { RefCell::new(None) };
    /// Where the cursor was on pages shown (page key, focus number), newest last. Kept across hand-offs to other
    /// screens; forgotten when the guided pages end (the page window goes and nothing else is open).
    static HISTORY: RefCell<Vec<(String, usize)>> = const { RefCell::new(Vec::new()) };
}

fn with_art<R>(app: &AppState, f: impl FnOnce(&mut ArtState) -> R) -> R {
    ART.with(|a| {
        let mut a = a.borrow_mut();
        if a.as_ref().is_none_or(|s| s.cache_dir != app.paths.cache_dir) {
            *a = Some(ArtState { cache_dir: app.paths.cache_dir.clone(), loader: ThumbLoader::new(app.paths.cache_dir.clone()), images: HashMap::new() });
        }
        f(a.as_mut().expect("just filled"))
    })
}

fn to_image(thumb: &crate::fork::art::thumbs::Thumb) -> slint::Image {
    slint::Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&thumb.rgba, thumb.width, thumb.height))
}

/// The QR code of a card as a picture: 10 pixels a square (it is shown at about 300 pixels, so it is only ever
/// scaled down) and a white border of 4 squares around it.
fn qr_image(qr: &crate::fork::cloud::qr::QrMatrix) -> slint::Image {
    let (width, height, rgba) = qr.rgba(10, 4);
    slint::Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, width, height))
}

fn model<T: Clone + 'static>(items: Vec<T>) -> slint::ModelRc<T> {
    slint::ModelRc::new(slint::VecModel::from(items))
}

fn image_of(images: &HashMap<String, Option<slint::Image>>, art: &Option<ArtRef>) -> slint::Image {
    art.as_ref().and_then(|a| images.get(&a.folder)).cloned().flatten().unwrap_or_default()
}

/// Sets the card and the tiles from the covers known now.
fn apply_art<A: Clone>(d: &SetupWizardDialog, page: &Page<A>, images: &HashMap<String, Option<slint::Image>>) {
    if let Some(card) = &page.card {
        d.set_card(WizardCard {
            image: card.qr.as_ref().map_or_else(|| image_of(images, &card.art), qr_image),
            square: card.qr.is_some(),
            title: card.title.clone().into(),
            lines: model(card.lines.iter().map(|l| l.clone().into()).collect::<Vec<slint::SharedString>>()),
        });
    }
    d.set_tiles(model(
        page.tiles.iter().enumerate().map(|(i, t)| WizardTile { image: image_of(images, &t.art), label: t.label.clone().into(), note: t.note.clone().into(), focus: page::tile_number(page, i) }).collect::<Vec<_>>(),
    ));
}

/// Shows what is cached now, asks for the rest and, while covers are still on their way, swaps them in as
/// they arrive. One timer serves the page on screen; a new page replaces it.
fn load_art<A: Clone + 'static>(app: &Rc<AppState>, d: &SetupWizardDialog, cur: &Current<A>) {
    let page = page_now(cur);
    let refs = page.art_refs();
    let pending = |images: &HashMap<String, Option<slint::Image>>| refs.iter().any(|r| !images.contains_key(&r.folder));
    let still = with_art(app, |s| {
        apply_art(d, &page, &s.images);
        s.loader.request(&refs);
        pending(&s.images)
    });
    if !still {
        return ART_TIMER.with(|t| t.stop());
    }
    let (app, weak, cur) = (app.clone(), d.as_weak(), cur.clone());
    ART_TIMER.with(|t| {
        t.start(slint::TimerMode::Repeated, ART_TICK, move || {
            let Some(d) = weak.upgrade() else { return ART_TIMER.with(|t| t.stop()) };
            let page = page_now(&cur);
            let refs = page.art_refs();
            let done = with_art(&app, |s| {
                for loaded in s.loader.drain() {
                    s.images.insert(loaded.folder, loaded.thumb.as_ref().map(to_image));
                }
                apply_art(&d, &page, &s.images);
                !refs.iter().any(|r| !s.images.contains_key(&r.folder))
            });
            if done {
                ART_TIMER.with(|t| t.stop());
            }
        });
    });
}

fn fill<A: Clone>(d: &SetupWizardDialog, page: &Page<A>) {
    d.set_dialog_title(page.window_title.clone().into());
    d.set_large(page.large);
    d.set_header_text(page.header.clone().into());
    d.set_step_text(page.step_text.clone().into());
    d.set_step_count(page.step_count as i32);
    d.set_step_index(page.step_number as i32 - 1);
    d.set_page_title(page.title.clone().into());
    d.set_has_input(page.input.is_some());
    d.set_hint_text(page::hints(page).into());
    fill_body(d, page);
}

/// Marks focus number `n` as selected: the window draws it, and scrolls to it when it is an option.
fn select<A: Clone>(d: &SetupWizardDialog, page: &Page<A>, focus: &Cell<usize>, n: usize) {
    focus.set(n);
    let option = match page::focus_at(page, n) {
        Some(Focus::Option(i)) => i as i32,
        _ => -1,
    };
    d.set_selected_option(option);
    d.set_selected_index(n as i32);
}

/// The page on screen. Pages with a text box replace it as the text changes (a filtered list), so everything
/// that reads the page reads it from here at the moment it needs it.
type Current<A> = Rc<RefCell<Rc<Page<A>>>>;

fn page_now<A>(cur: &Current<A>) -> Rc<Page<A>> {
    cur.borrow().clone()
}

/// Fills the part of the window that follows the text: the paragraphs, the facts, the card, the tiles, the
/// options, the footer. (`fill` also sets the window's own texts, which do not change with the text box.)
fn fill_body<A: Clone>(d: &SetupWizardDialog, page: &Page<A>) {
    d.set_paragraphs(model(page.paragraphs.iter().map(|p| p.clone().into()).collect::<Vec<slint::SharedString>>()));
    d.set_facts(model(page.facts.iter().enumerate().map(|(i, f)| WizardFact { text: f.clone().into(), mark: page.mark_of(i).code() }).collect::<Vec<_>>()));
    let (kind, value) = match page.progress {
        None => (0, 0.0),
        Some(p) => p.fraction().map_or((2, 0.0), |f| (1, f)),
    };
    d.set_progress_kind(kind);
    d.set_facts_marked(page.marks.iter().any(|m| *m != crate::fork::page::Mark::None));
    d.set_progress_value(value);
    d.set_has_card(page.card.is_some());
    d.set_options(model(
        page.options
            .iter()
            .enumerate()
            .map(|(i, o)| WizardOption {
                title: o.title.clone().into(),
                detail: o.detail.clone().into(),
                kind: i32::from(o.choice.is_some()),
                chosen: o.choice == Some(true),
                enabled: o.enabled,
                recommended: o.recommended,
                focus: page::option_number(page, i),
                icon: page.icon_of(i).code(),
                toggle: page.toggle_of(i).map_or(0, |on| if on { 2 } else { 1 }),
            })
            .collect::<Vec<_>>(),
    ));
    d.set_footnote(page.footnote.clone().into());
    let label = |slot: usize| page.buttons[slot].as_ref().map(|b| b.label.clone()).unwrap_or_default();
    d.set_left_text(label(page::LEFT).into());
    d.set_middle_text(label(page::MIDDLE).into());
    d.set_extra_text(label(page::EXTRA).into());
    d.set_right_text(label(page::RIGHT).into());
    d.set_primary_slot(page.primary.map_or(-1, |s| s as i32));
    d.set_focus_left(page::button_number(page, page::LEFT));
    d.set_focus_middle(page::button_number(page, page::MIDDLE));
    d.set_focus_extra(page::button_number(page, page::EXTRA));
    d.set_focus_right(page::button_number(page, page::RIGHT));
    d.set_input_focus(page::input_number(page));
}

/// What a page answers to, whatever its action type: the window's callbacks reach the page on screen through
/// this, so a new page (of any owner) can take the window over.
trait Binding {
    fn option(&self, i: i32);
    fn tile(&self, i: i32);
    fn button(&self, slot: i32);
    fn hover(&self, n: i32);
    fn move_by(&self, dx: i32, dy: i32);
    fn activate(&self);
    fn input(&self, text: &str);
    fn back(&self);
    fn exit(&self);
    /// The page's key and where the cursor is on it (for "B returns to where you were").
    fn place(&self) -> (String, usize);
}

struct Bound<A> {
    app: Rc<AppState>,
    router: Router,
    weak: slint::Weak<SetupWizardDialog>,
    cur: Current<A>,
    focus: Rc<Cell<usize>>,
    handlers: Handlers<A>,
}

impl<A: Clone + 'static> Binding for Bound<A> {
    fn option(&self, i: i32) {
        // A disabled option does nothing.
        let action = page_now(&self.cur).options.get(i as usize).filter(|o| o.enabled).map(|o| o.action.clone());
        if let Some(action) = action {
            (self.handlers.on_action)(&self.app, &self.router, action);
        }
    }

    fn tile(&self, i: i32) {
        let action = page_now(&self.cur).tiles.get(i as usize).and_then(|t| t.action.clone());
        if let Some(action) = action {
            (self.handlers.on_action)(&self.app, &self.router, action);
        }
    }

    fn button(&self, slot: i32) {
        let action = page_now(&self.cur).buttons.get(slot as usize).and_then(Option::as_ref).map(|b| b.action.clone());
        if let Some(action) = action {
            (self.handlers.on_action)(&self.app, &self.router, action);
        }
    }

    fn hover(&self, n: i32) {
        if let (Some(d), true) = (self.weak.upgrade(), n >= 0) {
            select(&d, &page_now(&self.cur), &self.focus, n as usize);
        }
    }

    fn move_by(&self, dx: i32, dy: i32) {
        let page = page_now(&self.cur);
        let next = page::move_focus(&page, self.focus.get(), dx, dy);
        if let Some(d) = self.weak.upgrade() {
            select(&d, &page, &self.focus, next);
        }
    }

    fn activate(&self) {
        let page = page_now(&self.cur);
        // A on the text box is not a choice: it moves on to the choices.
        if page::focus_at(&page, self.focus.get()) == Some(Focus::Input) {
            let next = page::move_focus(&page, self.focus.get(), 0, 1);
            if let Some(d) = self.weak.upgrade() {
                select(&d, &page, &self.focus, next);
            }
        } else if let Some(action) = page::action_at(&page, self.focus.get()) {
            (self.handlers.on_action)(&self.app, &self.router, action);
        }
    }

    fn input(&self, text: &str) {
        // Every edit of the text box goes to the owner, which may answer with the page as it should now read.
        let Some(on_input) = &self.handlers.on_input else { return };
        let Some(new_page) = on_input(&self.app, &self.router, text) else { return };
        let Some(d) = self.weak.upgrade() else { return };
        let new_page = Rc::new(new_page);
        *self.cur.borrow_mut() = new_page.clone();
        fill_body(&d, &new_page);
        load_art(&self.app, &d, &self.cur);
        // The cursor stays in the box, and the list below starts from the top again.
        select(&d, &new_page, &self.focus, page::input_number(&new_page).max(0) as usize);
    }

    fn back(&self) {
        (self.handlers.on_back)(&self.app, &self.router);
    }

    fn exit(&self) {
        (self.handlers.on_exit)(&self.app, &self.router);
    }

    fn place(&self) -> (String, usize) {
        (page_key(&page_now(&self.cur)), self.focus.get())
    }
}

/// Names a page for the focus memory: the same window title, header, counter ("Port 2 of 5"), question and subject
/// (the card's port, the first cover) is the same page. A card drawn again for another port is another page.
fn page_key<A>(page: &Page<A>) -> String {
    let card = page.card.as_ref().map_or("", |c| c.title.as_str());
    let tile = page.tiles.first().map_or("", |t| t.label.as_str());
    format!("{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{card}\u{1f}{tile}", page.window_title, page.header, page.step_text, page.title)
}

/// The page on the window with this serial, when there is one (not parked, not replaced by a newer window).
fn bound(serial: u64) -> Option<Rc<dyn Binding>> {
    HOST.with(|h| h.borrow().bound(serial))
}

/// Notes where the cursor is on the page being left, so coming back to it finds the same place.
fn remember_current() {
    let Some(place) = HOST.with(|h| h.borrow().page.as_ref().map(|(_, b)| b.place())) else { return };
    remember(place);
}

fn remember((key, focus): (String, usize)) {
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        h.retain(|(k, _)| *k != key);
        h.push((key, focus));
        let extra = h.len().saturating_sub(HISTORY_LEN);
        h.drain(..extra);
    });
}

fn recalled(key: &str) -> Option<usize> {
    HISTORY.with(|h| h.borrow().iter().rev().find(|(k, _)| k == key).map(|(_, f)| *f))
}

/// Where the cursor starts on `page`: where it was when the page was shown before in these guided pages (B, a footer
/// Back button, a page drawn again after a hand-off), when that place still exists; else the page's own start.
fn starting_focus<A>(page: &Page<A>) -> usize {
    match recalled(&page_key(page)) {
        Some(n) if n < page::focusables(page).len() => n,
        _ => page::initial_focus(page),
    }
}

/// Wires a newly built window's callbacks once: each reaches whatever page is on it now.
fn wire(dialog: &SetupWizardDialog, serial: u64) {
    dialog.on_option_requested(move |i| {
        if let Some(b) = bound(serial) {
            b.option(i);
        }
    });
    dialog.on_tile_requested(move |i| {
        if let Some(b) = bound(serial) {
            b.tile(i);
        }
    });
    dialog.on_button_requested(move |slot| {
        if let Some(b) = bound(serial) {
            b.button(slot);
        }
    });
    dialog.on_nav_hovered(move |n| {
        if let Some(b) = bound(serial) {
            b.hover(n);
        }
    });
    dialog.on_move_requested(move |dx, dy| {
        if let Some(b) = bound(serial) {
            b.move_by(dx, dy);
        }
    });
    // A and Return, B and Escape: one physical press acts once (`press_echo`; the pad's own press passes).
    dialog.on_activate_requested(move || {
        if let Some(b) = bound(serial).filter(|_| press_echo::key(Press::Choose)) {
            b.activate();
        }
    });
    dialog.on_input_edited(move |text| {
        if let Some(b) = bound(serial) {
            b.input(text.as_str());
        }
    });
    dialog.on_close_requested(move || {
        if let Some(b) = bound(serial).filter(|_| press_echo::key(Press::Back)) {
            b.back();
        }
    });
    dialog.on_exit_requested(move || {
        if let Some(b) = bound(serial) {
            b.exit();
        }
    });
}

/// After `finish_dialog_open` (which makes the window system's close button close the dialog): that button
/// leaves like the X does instead. When the owner answers by showing another page here, the window stays.
fn wire_window_close(dialog: &SetupWizardDialog, serial: u64) {
    dialog.window().on_close_requested(move || {
        if let Some(b) = bound(serial) {
            b.exit();
        }
        if bound(serial).is_some() {
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });
}

/// `close_current_dialog` found the page window in the slot (it has already emptied the slot). Instead of
/// hiding it now, keep it up for a moment: a page shown meanwhile takes it back (one window from page to page,
/// even across a hand-off that closes first). Otherwise it is hidden then, and the main window gets the focus
/// as a normal close gives it. False when the window is not the page window (the caller closes it normally).
pub(crate) fn park(app: &Rc<AppState>, dialog: &SetupWizardDialog) -> bool {
    let Some(serial) = WIN.with(|w| w.borrow().as_ref().filter(|(_, d)| same_window(d, dialog)).map(|(s, _)| *s)) else { return false };
    remember_current();
    let Some(token) = HOST.with(|h| h.borrow_mut().park(serial)) else { return false };
    let app = app.clone();
    slint::Timer::single_shot(PARK, move || {
        if !HOST.with(|h| h.borrow_mut().expire(token)) {
            return;
        }
        // The main window's layer waited through the park; now it follows whatever is open (nothing, or a hand-off).
        super::dialog_layer::sync(&app);
        let Some((_, dialog)) = WIN.with(|w| w.borrow_mut().take()) else { return };
        let _ = dialog.window().hide();
        ART_TIMER.with(|t| t.stop());
        // What `close_current_dialog` does after hiding a dialog. When another dialog opened meanwhile (a hand-off) it
        // is in front already and keeps the keyboard, so the main window is left alone, and the cursor memory is kept
        // for when the pages come back. With nothing open the guided pages have ended.
        if !dialog_is_open(&app) {
            HISTORY.with(|h| h.borrow_mut().clear());
            if let Some(native) = chrome::native_window(app.window().window()) {
                chrome::force_foreground_window(native);
            }
            let main = app.window();
            main.set_refocus_trigger(!main.get_refocus_trigger());
        }
    });
    true
}

/// True while the page window waits for its next page (see [`park`]); the main window's layer stays as it was meanwhile.
pub(crate) fn parked() -> bool {
    HOST.with(|h| h.borrow().parked.is_some())
}

fn same_window(a: &SetupWizardDialog, b: &SetupWizardDialog) -> bool {
    std::ptr::eq(a.window(), b.window())
}

/// True while the page window is open and showing a page (not parked).
fn page_window_in_slot(app: &AppState) -> bool {
    let in_slot = app.dialog_nav.dialogs.try_borrow().is_ok_and(|s| matches!(*s, DialogSlot::SetupWizard(_)));
    in_slot && HOST.with(|h| h.borrow().on_screen())
}

fn page_window() -> Option<(u64, SetupWizardDialog)> {
    WIN.with(|w| w.borrow().as_ref().map(|(s, d)| (*s, d.clone_strong())))
}

/// The window to draw the next page in: the one on screen, the parked one, or a new one. The flag says it was
/// already on screen with a page (nothing to register again).
fn take_window(app: &Rc<AppState>, router: &Router) -> Option<(SetupWizardDialog, u64, bool)> {
    if page_window_in_slot(app) {
        if let Some((serial, dialog)) = page_window() {
            return Some((dialog, serial, true));
        }
    }
    if HOST.with(|h| h.borrow().parked.is_some()) {
        if let Some((serial, dialog)) = page_window() {
            // Whatever else is open (a message shown in the meantime) gives way to the page.
            close_current_dialog(app, router);
            HOST.with(|h| h.borrow_mut().retake());
            return Some((dialog, serial, false));
        }
    }
    // A page window that is still up but no longer in the slot (something replaced it without closing it) goes.
    if let Some((_, old)) = WIN.with(|w| w.borrow_mut().take()) {
        let _ = old.window().hide();
    }
    let dialog = build_setup_wizard_dialog(app, router)?;
    let serial = HOST.with(|h| h.borrow_mut().new_window());
    wire(&dialog, serial);
    WIN.with(|w| *w.borrow_mut() = Some((serial, dialog.clone_strong())));
    Some((dialog, serial, false))
}

/// A page on screen that its owner can replace while it stays open (a progress page).
pub(crate) struct PageHandle<A> {
    app: Rc<AppState>,
    weak: slint::Weak<SetupWizardDialog>,
    cur: Current<A>,
    focus: Rc<Cell<usize>>,
    generation: u64,
}

impl<A: Clone + 'static> PageHandle<A> {
    /// True while this page is still the one on screen (no other page or dialog replaced it).
    pub(crate) fn is_open(&self) -> bool {
        HOST.with(|h| h.borrow().is_current(self.generation)) && self.weak.upgrade().is_some_and(|d| d.window().is_visible())
    }

    /// Draws `page` in its place; the cursor stays where it was when that place still exists. False when the
    /// page is gone (something else replaced it), so the owner can stop updating.
    pub(crate) fn update(&self, page: Page<A>) -> bool {
        if !self.is_open() {
            return false;
        }
        let Some(d) = self.weak.upgrade() else { return false };
        let page = Rc::new(page);
        *self.cur.borrow_mut() = page.clone();
        fill(&d, &page);
        load_art(&self.app, &d, &self.cur);
        let n = self.focus.get().min(page::focusables(&page).len().saturating_sub(1));
        select(&d, &page, &self.focus, n);
        super::dialog_layer::sync(&self.app);
        true
    }
}

/// Shows the page: in the page window when it is open (or parked), otherwise in a new one, replacing whatever
/// dialog is open. False when the window could not be made.
pub(crate) fn show_page<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, page: Page<A>, handlers: Handlers<A>) -> bool {
    show_page_handle(app, router, page, handlers).is_some()
}

/// [`show_page`], giving back the handle that replaces the page later.
pub(crate) fn show_page_handle<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, page: Page<A>, handlers: Handlers<A>) -> Option<PageHandle<A>> {
    remember_current();
    let (dialog, serial, on_screen) = take_window(app, router)?;
    let cur: Current<A> = Rc::new(RefCell::new(Rc::new(page)));
    let focus = Rc::new(Cell::new(0usize));
    let first = page_now(&cur);
    fill(&dialog, &first);
    dialog.set_input_placeholder(first.input.as_ref().map(|i| i.placeholder.clone()).unwrap_or_default().into());
    dialog.set_input_text(first.input.as_ref().map(|i| i.text.clone()).unwrap_or_default().into());
    load_art(app, &dialog, &cur);
    select(&dialog, &first, &focus, starting_focus(&first));
    // A new page: the body scrolls back to the top, a resting pointer cannot take the highlight, and the
    // keyboard goes to the page (or its text box when that is where the cursor starts).
    dialog.invoke_page_shown();

    let binding: Rc<dyn Binding> = Rc::new(Bound { app: app.clone(), router: router.clone(), weak: dialog.as_weak(), cur: cur.clone(), focus: focus.clone(), handlers });
    let generation = HOST.with(|h| h.borrow_mut().show(serial, binding));

    if !on_screen {
        let _ = dialog.show();
        finish_dialog_open(app, router, DialogSlot::SetupWizard(dialog.clone_strong()));
        wire_window_close(&dialog, serial);
    }
    // The main window's layer names the page now on screen.
    super::dialog_layer::sync(app);
    Some(PageHandle { app: app.clone(), weak: dialog.as_weak(), cur, focus, generation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::page::{PageButton, PageOption};

    fn page(title: &str, options: usize) -> Page<u8> {
        let mut p = Page::<u8> { window_title: "Setup wizard".into(), title: title.into(), ..Page::default() };
        p.options = (0..options).map(|i| PageOption { title: format!("o{i}"), detail: String::new(), choice: None, enabled: true, recommended: false, action: i as u8 }).collect();
        p.buttons[page::RIGHT] = Some(PageButton { label: "Next".into(), action: 99 });
        p.initial = Focus::Button(page::RIGHT);
        p
    }

    fn clear() {
        HISTORY.with(|h| h.borrow_mut().clear());
    }

    /// A page that only says it was asked something.
    struct Stub;
    impl Binding for Stub {
        fn option(&self, _: i32) {}
        fn tile(&self, _: i32) {}
        fn button(&self, _: i32) {}
        fn hover(&self, _: i32) {}
        fn move_by(&self, _: i32, _: i32) {}
        fn activate(&self) {}
        fn input(&self, _: &str) {}
        fn back(&self) {}
        fn exit(&self) {}
        fn place(&self) -> (String, usize) {
            ("stub".into(), 0)
        }
    }

    fn stub() -> Rc<dyn Binding> {
        Rc::new(Stub)
    }

    #[test]
    fn a_parked_window_answers_nothing_and_its_old_page_handle_is_stale() {
        let mut h = Host::default();
        let w = h.new_window();
        let first = h.show(w, stub());
        assert!(h.bound(w).is_some() && h.is_current(first) && h.on_screen());
        let token = h.park(w).expect("the page window parks");
        assert!(h.bound(w).is_none(), "a key that reaches the parked window does nothing");
        assert!(!h.is_current(first), "a progress page shown before the park stops updating");
        assert!(!h.on_screen());
        assert!(h.park(w).is_none(), "a second close of the same window does not park it twice");
        assert!(h.park(w + 7).is_none(), "another window is closed normally");
        assert!(h.expire(token), "and with no page in time the park ends it");
    }

    #[test]
    fn a_page_shown_while_parked_takes_the_window_back_and_the_park_timer_then_does_nothing() {
        let mut h = Host::default();
        let w = h.new_window();
        h.show(w, stub());
        let token = h.park(w).unwrap();
        assert_eq!(h.retake(), Some(w), "the same window, not a new one");
        assert!(h.parked.is_none());
        let second = h.show(w, stub());
        assert!(h.bound(w).is_some() && h.is_current(second) && h.on_screen());
        assert!(!h.expire(token), "the 40 ms timer finds the window taken back and leaves it");
        assert_eq!(h.window, Some(w));
    }

    #[test]
    fn with_no_page_in_time_the_park_ends_the_window() {
        let mut h = Host::default();
        let w = h.new_window();
        h.show(w, stub());
        let token = h.park(w).unwrap();
        assert!(h.expire(token));
        assert_eq!(h.window, None);
        assert!(h.retake().is_none(), "nothing left to take back");
        assert!(!h.expire(token), "a late second timer does nothing");
        let w2 = h.new_window();
        assert_ne!(w, w2, "a new window gets a new serial, so the old window's callbacks reach nothing");
        h.show(w2, stub());
        assert!(h.bound(w).is_none() && h.bound(w2).is_some());
    }

    #[test]
    fn every_page_shown_makes_older_handles_stale() {
        let mut h = Host::default();
        let w = h.new_window();
        let a = h.show(w, stub());
        let b = h.show(w, stub());
        assert!(!h.is_current(a) && h.is_current(b));
    }

    #[test]
    fn the_cursor_comes_back_to_where_it_was_on_a_page_seen_before() {
        clear();
        let first = page("Where are your ports?", 3);
        assert_eq!(starting_focus(&first), page::initial_focus(&first), "the first time it starts where the page says");
        remember((page_key(&first), 2));
        assert_eq!(starting_focus(&first), 2, "back to the page (B, a Back button, after a hand-off): the third option again");
    }

    #[test]
    fn a_remembered_place_that_no_longer_exists_falls_back_to_the_start() {
        clear();
        let before = page("Your ports", 4);
        remember((page_key(&before), 4));
        let now = page("Your ports", 1);
        assert_eq!(starting_focus(&now), page::initial_focus(&now));
    }

    #[test]
    fn an_unseen_page_starts_where_it_says() {
        clear();
        remember((page_key(&page("Other", 3)), 1));
        let p = page("Never shown", 3);
        assert_eq!(starting_focus(&p), page::initial_focus(&p));
    }

    #[test]
    fn the_memory_keeps_the_latest_place_per_page_and_stays_small() {
        clear();
        let p = page("Steam", 3);
        remember((page_key(&p), 0));
        remember((page_key(&p), 1));
        assert_eq!(recalled(&page_key(&p)), Some(1));
        for i in 0..(HISTORY_LEN + 5) {
            remember((format!("k{i}"), i));
        }
        assert_eq!(HISTORY.with(|h| h.borrow().len()), HISTORY_LEN);
        assert_eq!(recalled(&page_key(&p)), None, "the oldest pages are forgotten first");
    }

    #[test]
    fn pages_differ_by_window_title_and_question() {
        let a = page("Steam", 1);
        let mut b = a.clone();
        b.window_title = "Import ports: found".into();
        assert_ne!(page_key(&a), page_key(&b));
        let mut c = a.clone();
        c.paragraphs = vec!["a new count".into()];
        assert_eq!(page_key(&a), page_key(&c), "the same question with fresh numbers is the same page");
        let mut d = a.clone();
        d.step_text = "Port 2 of 5".into();
        assert_ne!(page_key(&a), page_key(&d), "the next card with the same question is another page");
        // "Pick another port" on a card, then the card drawn again for the port picked: the cursor starts on that
        // page's own choice (Yes), not where it was for the old port.
        let mut card = a.clone();
        card.card = Some(crate::fork::page::Card { art: None, qr: None, title: "Dusklight".into(), lines: vec![] });
        let mut other = card.clone();
        other.card.as_mut().unwrap().title = "Ship of Harkinian".into();
        assert_ne!(page_key(&card), page_key(&other));
        clear();
        remember((page_key(&card), 1));
        assert_eq!(starting_focus(&other), page::initial_focus(&other));
    }
}
