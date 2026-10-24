//! Fork: finishing an installed update whose release names its files with the version (see
//! `fork::versioned`). Called on the UI thread when an install is done.

use super::state::AppState;
use crate::fork::port_mover::steam::SteamCtx;
use crate::fork::settings::{ForkSettings, FILE_NAME};
use crate::fork::store::SystemSteamCheck;
use crate::fork::versioned::{self, FinishInput};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"].iter().find_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Points the launch target and Steam at the new files and moves the old ones to `.previous/`.
/// Returns the text to show, or `None` when the update superseded nothing.
pub(crate) fn after_install(app: &Rc<AppState>, key: &str, tag: Option<&str>) -> Option<String> {
    let port = app.find_port(key)?;
    let game_dir = crate::core::path_safety::safe_join(&app.paths.library_dir(), &port.folder).ok()?;
    let favorite = app.state.borrow().get(key).and_then(|i| i.favorite_exe.clone());
    let home = home_dir();
    let settings = ForkSettings::load(&app.paths.config_dir.join(FILE_NAME));
    let steam_check = SystemSteamCheck::new();
    let ctx = home.as_ref().map(|home| SteamCtx {
        home,
        config_dir: &app.paths.config_dir,
        settings: &settings,
        steam: &steam_check,
        game_mode: crate::fork::steam_queue::game_mode_from_env(),
        now: SystemTime::now(),
    });
    let outcome = versioned::finish_update(&FinishInput { port: &port, game_dir: &game_dir, favorite: favorite.as_deref(), steam: ctx.as_ref() });
    if let Some(new_favorite) = &outcome.new_favorite {
        app.state.borrow_mut().set_favorite_exe(key, Some(new_favorite.clone()));
    }
    for (m, folder) in &outcome.moved {
        eprintln!("versioned update: {} moved {} to .previous/{folder} (replaced by {})", port.folder, m.old, m.new);
    }
    for (name, why) in &outcome.kept {
        eprintln!("versioned update: {} kept {name} in place: {why}", port.folder);
    }
    outcome.summary(port.display_name(), &port.folder, tag)
}
