//! Draws a [`Page`] in the guided-screen window (`ui/fork/setup-wizard.slint`): header, title, text, a card,
//! cover tiles, big options and footer buttons. The setup wizard and the Import ports cards both show their
//! pages through [`show_page`]; what a chosen item means is theirs to say (the `A` of the page).
//!
//! Only the controller's D-pad, A and B (and the same keys on a keyboard, or the mouse) reach the window:
//! the D-pad moves between options and buttons, A chooses, B goes back one page (`on_back`), and the title
//! bar's X or the window's own close leaves the whole thing (`on_exit`).
//!
//! Covers arrive after the page is up: they are fetched and decoded on a background thread
//! (`fork::art::thumbs`) and a timer swaps them in, so a page never waits on the network. Until a cover is
//! there (and when there is none) the port's name is shown on a plain tile.

use super::dialogs::{build_setup_wizard_dialog, finish_dialog_open, DialogSlot};
use super::state::AppState;
use crate::fork::art::thumbs::{ArtRef, ThumbLoader};
use crate::fork::page::{self, Focus, Page};
use crate::ui::gamepad_router::GamepadRouter;
use crate::{SetupWizardDialog, WizardCard, WizardOption, WizardTile};
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

/// The covers seen so far: `Some(image)`, or `None` when the cover could not be had (keep the name tile).
struct ArtState {
    cache_dir: PathBuf,
    loader: ThumbLoader,
    images: HashMap<String, Option<slint::Image>>,
}

thread_local! {
    static ART: RefCell<Option<ArtState>> = const { RefCell::new(None) };
    static ART_TIMER: slint::Timer = slint::Timer::default();
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

/// The QR code of a card as a picture: 6 pixels a square and a white border of 4 squares around it.
fn qr_image(qr: &crate::fork::cloud::qr::QrMatrix) -> slint::Image {
    let (width, height, rgba) = qr.rgba(6, 4);
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
    d.set_facts(model(page.facts.iter().map(|f| f.clone().into()).collect::<Vec<slint::SharedString>>()));
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

/// A page on screen that its owner can replace while it stays open (a progress page).
pub(crate) struct PageHandle<A> {
    app: Rc<AppState>,
    weak: slint::Weak<SetupWizardDialog>,
    cur: Current<A>,
    focus: Rc<Cell<usize>>,
}

impl<A: Clone + 'static> PageHandle<A> {
    /// True while the window is still on screen (nothing else replaced it).
    pub(crate) fn is_open(&self) -> bool {
        self.weak.upgrade().is_some_and(|d| d.window().is_visible())
    }

    /// Draws `page` in the same window; the cursor stays where it was when that place still exists. False when the
    /// window is gone (something else replaced it), so the owner can stop updating.
    pub(crate) fn update(&self, page: Page<A>) -> bool {
        let Some(d) = self.weak.upgrade() else { return false };
        if !d.window().is_visible() {
            return false;
        }
        let page = Rc::new(page);
        *self.cur.borrow_mut() = page.clone();
        fill(&d, &page);
        load_art(&self.app, &d, &self.cur);
        let n = self.focus.get().min(page::focusables(&page).len().saturating_sub(1));
        select(&d, &page, &self.focus, n);
        true
    }
}

/// Opens the page, replacing whatever dialog is open. False when the window could not be made.
pub(crate) fn show_page<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, page: Page<A>, handlers: Handlers<A>) -> bool {
    show_page_handle(app, router, page, handlers).is_some()
}

/// [`show_page`], giving back the handle that replaces the page later.
pub(crate) fn show_page_handle<A: Clone + 'static>(app: &Rc<AppState>, router: &Router, page: Page<A>, handlers: Handlers<A>) -> Option<PageHandle<A>> {
    let cur: Current<A> = Rc::new(RefCell::new(Rc::new(page)));
    let Some(dialog) = build_setup_wizard_dialog(app, router) else { return None };
    let focus = Rc::new(Cell::new(0usize));
    {
        let first = page_now(&cur);
        fill(&dialog, &first);
        if let Some(input) = &first.input {
            dialog.set_input_placeholder(input.placeholder.clone().into());
            dialog.set_input_text(input.text.clone().into());
        }
        load_art(app, &dialog, &cur);
        select(&dialog, &first, &focus, page::initial_focus(&first));
    }

    {
        let (app, router, cur, h) = (app.clone(), router.clone(), cur.clone(), handlers.clone());
        dialog.on_option_requested(move |i| {
            // A disabled option does nothing.
            if let Some(option) = page_now(&cur).options.get(i as usize).filter(|o| o.enabled) {
                (h.on_action)(&app, &router, option.action.clone());
            }
        });
    }
    {
        let (app, router, cur, h) = (app.clone(), router.clone(), cur.clone(), handlers.clone());
        dialog.on_tile_requested(move |i| {
            if let Some(action) = page_now(&cur).tiles.get(i as usize).and_then(|t| t.action.clone()) {
                (h.on_action)(&app, &router, action);
            }
        });
    }
    {
        let (app, router, cur, h) = (app.clone(), router.clone(), cur.clone(), handlers.clone());
        dialog.on_button_requested(move |slot| {
            if let Some(button) = page_now(&cur).buttons.get(slot as usize).and_then(Option::as_ref) {
                (h.on_action)(&app, &router, button.action.clone());
            }
        });
    }
    {
        let (weak, cur, focus) = (dialog.as_weak(), cur.clone(), focus.clone());
        dialog.on_nav_hovered(move |n| {
            if let (Some(d), true) = (weak.upgrade(), n >= 0) {
                select(&d, &page_now(&cur), &focus, n as usize);
            }
        });
    }
    {
        let (weak, cur, focus) = (dialog.as_weak(), cur.clone(), focus.clone());
        dialog.on_move_requested(move |dx, dy| {
            let page = page_now(&cur);
            let next = page::move_focus(&page, focus.get(), dx, dy);
            if let Some(d) = weak.upgrade() {
                select(&d, &page, &focus, next);
            }
        });
    }
    {
        let (app, router, cur, focus, h, weak) = (app.clone(), router.clone(), cur.clone(), focus.clone(), handlers.clone(), dialog.as_weak());
        dialog.on_activate_requested(move || {
            let page = page_now(&cur);
            // A on the text box is not a choice: it moves on to the choices.
            if page::focus_at(&page, focus.get()) == Some(Focus::Input) {
                let next = page::move_focus(&page, focus.get(), 0, 1);
                if let Some(d) = weak.upgrade() {
                    select(&d, &page, &focus, next);
                }
            } else if let Some(action) = page::action_at(&page, focus.get()) {
                (h.on_action)(&app, &router, action);
            }
        });
    }
    {
        // Every edit of the text box goes to the owner, which may answer with the page as it should now read.
        let (app, router, cur, focus, h, weak) = (app.clone(), router.clone(), cur.clone(), focus.clone(), handlers.clone(), dialog.as_weak());
        dialog.on_input_edited(move |text| {
            let Some(on_input) = &h.on_input else { return };
            let Some(new_page) = on_input(&app, &router, text.as_str()) else { return };
            let Some(d) = weak.upgrade() else { return };
            let new_page = Rc::new(new_page);
            *cur.borrow_mut() = new_page.clone();
            fill_body(&d, &new_page);
            load_art(&app, &d, &cur);
            // The cursor stays in the box, and the list below starts from the top again.
            select(&d, &new_page, &focus, page::input_number(&new_page).max(0) as usize);
        });
    }
    {
        let (app, router, h) = (app.clone(), router.clone(), handlers.clone());
        dialog.on_close_requested(move || (h.on_back)(&app, &router));
    }
    {
        let (app, router, h) = (app.clone(), router.clone(), handlers.clone());
        dialog.on_exit_requested(move || (h.on_exit)(&app, &router));
    }
    let _ = dialog.show();
    let for_window = dialog.clone_strong();
    let weak = dialog.as_weak();
    let (app_for_handle, cur_for_handle) = (app.clone(), cur.clone());
    finish_dialog_open(app, router, DialogSlot::SetupWizard(dialog));
    // The window system's own close (a window manager's button, Alt+F4) leaves like the X does;
    // `finish_dialog_open` would only close the page, which its owner would then draw again.
    let (app, router) = (app.clone(), router.clone());
    for_window.window().on_close_requested(move || {
        (handlers.on_exit)(&app, &router);
        slint::CloseRequestResponse::HideWindow
    });
    Some(PageHandle { app: app_for_handle, weak, cur: cur_for_handle, focus })
}
