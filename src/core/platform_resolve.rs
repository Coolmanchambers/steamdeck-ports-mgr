
use serde_json::Value;
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use windows::core::GUID;
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::CoTaskMemFree;
#[cfg(target_os = "windows")]
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos,
    SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

#[cfg(target_os = "windows")]
pub fn get_platform_key() -> &'static str {
    "windows"
}

#[cfg(target_os = "linux")]
pub fn get_platform_key() -> &'static str {
    "linux"
}

pub fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn resolve_per_platform(value: &Value) -> Option<Value> {
    let Some(obj) = value.as_object() else {
        return Some(value.clone());
    };
    obj.get(get_platform_key()).or_else(|| obj.get("windows")).or_else(|| obj.values().next()).cloned()
}

#[cfg(target_os = "windows")]
const KNOWN_FOLDERS: &[(&str, GUID)] = &[
    ("Desktop", FOLDERID_Desktop),
    ("Documents", FOLDERID_Documents),
    ("Downloads", FOLDERID_Downloads),
    ("Music", FOLDERID_Music),
    ("Pictures", FOLDERID_Pictures),
    ("Videos", FOLDERID_Videos),
];

#[cfg(target_os = "windows")]
fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let pwstr = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let result = pwstr.to_string().ok();
        CoTaskMemFree(Some(pwstr.0 as *const _));
        result
    }
}

#[cfg(target_os = "windows")]
fn strip_known_folder_component(rest: &str) -> Option<(GUID, &str)> {
    let after_sep = rest.strip_prefix(['\\', '/'])?;
    KNOWN_FOLDERS.iter().find_map(|(name, guid)| {
        let (head, tail) = after_sep.split_at_checked(name.len())?;
        (head.eq_ignore_ascii_case(name) && (tail.is_empty() || tail.starts_with(['\\', '/']))).then_some((*guid, tail))
    })
}

#[cfg(target_os = "windows")]
fn known_folder_override<'a>(var_name: &str, after_var: &'a str) -> Option<(String, &'a str)> {
    if !var_name.eq_ignore_ascii_case("USERPROFILE") {
        return None;
    }
    let (guid, remainder) = strip_known_folder_component(after_var)?;
    Some((known_folder(&guid)?, remainder))
}

#[cfg(target_os = "linux")]
fn known_folder_override<'a>(_var_name: &str, _after_var: &'a str) -> Option<(String, &'a str)> {
    None
}

#[cfg(target_os = "windows")]
const HOME_VAR: &str = "USERPROFILE";
#[cfg(target_os = "linux")]
const HOME_VAR: &str = "HOME";

fn split_tilde<'a>(path: &'a str, home: Option<&str>) -> Option<(String, &'a str)> {
    let home = home.filter(|h| !h.is_empty())?;
    let rest = path.strip_prefix('~')?;
    (rest.is_empty() || rest.starts_with(['/', '\\'])).then(|| (home.to_string(), rest))
}

// Fork: only the tests still call the unchecked form; `save_location` needs to know about unset variables.
#[cfg(test)]
pub fn expand_env_path(path: &str) -> PathBuf {
    expand_env_path_checked(path).0
}

// Fork: the bool is true when a `%VAR%` was left as is because the variable is not set.
fn expand_env_path_checked(path: &str) -> (PathBuf, bool) {
    let mut unresolved = false;
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    if let Some((home, remainder)) = split_tilde(path, std::env::var(HOME_VAR).ok().as_deref()) {
        out.push_str(&home);
        rest = remainder;
    }
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after_percent = &rest[start + 1..];
        let Some(end) = after_percent.find('%') else {
            out.push('%');
            rest = after_percent;
            break;
        };
        let var_name = &after_percent[..end];
        let after_var = &after_percent[end + 1..];
        if let Some((folder, remainder)) = known_folder_override(var_name, after_var) {
            out.push_str(&folder);
            rest = remainder;
            continue;
        }
        match std::env::var(var_name) {
            Ok(value) => out.push_str(&value),
            Err(_) => {
                unresolved = true;
                out.push('%');
                out.push_str(var_name);
                out.push('%');
            }
        }
        rest = after_var;
    }
    out.push_str(rest);
    (PathBuf::from(out), unresolved)
}

/// Fork: where a port keeps its saves on this OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveLocation {
    /// No save field, or an empty one.
    NotListed,
    Known(PathBuf),
    /// Linux only: the catalog gives a save folder for another OS only (a per-OS value with no `linux`
    /// key), or a path whose `%VAR%` is not set here (`%APPDATA%`). Upstream used the Windows or
    /// Android value anyway, which pointed at a folder that cannot exist, so backups silently did nothing.
    UnknownOnLinux,
}

#[cfg(target_os = "linux")]
fn save_value_for_this_os(save_folder: &Value) -> Result<Option<Value>, ()> {
    let Some(obj) = save_folder.as_object() else {
        return Ok(Some(save_folder.clone()));
    };
    match obj.get("linux") {
        Some(v) => Ok(Some(v.clone())),
        None if obj.values().any(is_truthy) => Err(()),
        None => Ok(None),
    }
}

#[cfg(target_os = "windows")]
fn save_value_for_this_os(save_folder: &Value) -> Result<Option<Value>, ()> {
    Ok(resolve_per_platform(save_folder))
}

pub fn save_location(save_folder: &Value, game_dir: &Path) -> SaveLocation {
    let resolved = match save_value_for_this_os(save_folder) {
        Ok(v) => v,
        Err(()) => return SaveLocation::UnknownOnLinux,
    };
    let Some(s) = resolved.as_ref().and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return SaveLocation::NotListed;
    };
    let (expanded, unresolved) = expand_env_path_checked(s);
    if unresolved && cfg!(target_os = "linux") {
        return SaveLocation::UnknownOnLinux;
    }
    SaveLocation::Known(if expanded.is_absolute() { expanded } else { game_dir.join(expanded) })
}

pub fn resolve_save_folder(save_folder: &Value, game_dir: &Path) -> Option<PathBuf> {
    match save_location(save_folder, game_dir) {
        SaveLocation::Known(path) => Some(path),
        SaveLocation::NotListed | SaveLocation::UnknownOnLinux => None,
    }
}

pub fn resolve_preferred_asset(preferred_asset: &Value) -> Option<String> {
    let resolved = resolve_per_platform(preferred_asset)?;
    let s = resolved.as_str().filter(|s| !s.is_empty())?;
    #[cfg(target_os = "linux")]
    if !preferred_asset.is_object() && s.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolve_per_platform_valeur_simple_est_inchangee() {
        assert_eq!(resolve_per_platform(&json!("x")), Some(json!("x")));
    }

    #[test]
    fn resolve_per_platform_objet_vide_est_none() {
        assert_eq!(resolve_per_platform(&json!({})), None);
    }

    #[test]
    fn resolve_per_platform_cle_exacte_puis_repli_premiere_valeur() {
        let mut obj = serde_json::Map::new();
        obj.insert(get_platform_key().to_string(), json!("w"));
        obj.insert("other-os".to_string(), json!("o"));
        assert_eq!(resolve_per_platform(&Value::Object(obj)), Some(json!("w")));
        assert_eq!(resolve_per_platform(&json!({"other-os-1": "l", "other-os-2": "m"})), Some(json!("l")));
    }

    #[test]
    fn expand_env_path_etend_une_variable_connue() {
        std::env::set_var("PL_TEST_VAR", "C:\\Somewhere");
        assert_eq!(expand_env_path("%PL_TEST_VAR%\\save"), PathBuf::from("C:\\Somewhere\\save"));
        std::env::remove_var("PL_TEST_VAR");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn expand_env_path_userprofile_documents_utilise_le_dossier_connu() {
        let documents = known_folder(&FOLDERID_Documents).expect("SHGetKnownFolderPath(FOLDERID_Documents)");
        assert_eq!(expand_env_path("%USERPROFILE%\\Documents\\eternalsonata"), PathBuf::from(documents).join("eternalsonata"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn expand_env_path_userprofile_reconnait_les_autres_dossiers_connus() {
        let downloads = known_folder(&FOLDERID_Downloads).expect("SHGetKnownFolderPath(FOLDERID_Downloads)");
        assert_eq!(expand_env_path("%USERPROFILE%\\Downloads\\file.zip"), PathBuf::from(downloads).join("file.zip"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn expand_env_path_userprofile_documentsfoo_nest_pas_confondu_avec_documents() {
        let original = std::env::var("USERPROFILE").ok();
        std::env::set_var("USERPROFILE", "C:\\Users\\test");
        assert_eq!(expand_env_path("%USERPROFILE%\\Documentsfoo\\save"), PathBuf::from("C:\\Users\\test\\Documentsfoo\\save"));
        match original {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }

    #[test]
    fn expand_env_path_laisse_une_variable_inconnue_telle_quelle() {
        assert_eq!(expand_env_path("%PL_DOES_NOT_EXIST%\\save"), PathBuf::from("%PL_DOES_NOT_EXIST%\\save"));
    }

    #[test]
    fn expand_env_path_sans_variable_est_inchange() {
        assert_eq!(expand_env_path("save"), PathBuf::from("save"));
    }

    #[test]
    fn expand_env_path_pourcent_non_referme_est_conserve() {
        assert_eq!(expand_env_path("100%save"), PathBuf::from("100%save"));
    }

    #[test]
    fn split_tilde_ne_remplace_que_le_tilde_initial_seul() {
        assert_eq!(split_tilde("~/x", Some("/home/me")), Some(("/home/me".to_string(), "/x")));
        assert_eq!(split_tilde("~", Some("/home/me")), Some(("/home/me".to_string(), "")));
        assert_eq!(split_tilde("~other/x", Some("/home/me")), None);
        assert_eq!(split_tilde("a/~/x", Some("/home/me")), None);
        assert_eq!(split_tilde("~/x", None), None);
        assert_eq!(split_tilde("~/x", Some("")), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn expand_env_path_tilde_se_resout_sous_le_dossier_personnel() {
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(expand_env_path("~/.local/share/x"), PathBuf::from(&home).join(".local/share/x"));
        assert_eq!(expand_env_path("~"), PathBuf::from(&home));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn expand_env_path_home_entre_pourcents_marche_toujours() {
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(expand_env_path("%HOME%/.local/share/x"), PathBuf::from(&home).join(".local/share/x"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn resolve_save_folder_tilde_est_absolu_et_ne_se_colle_plus_au_dossier_du_jeu() {
        let home = std::env::var("HOME").expect("HOME");
        let save_folder = json!({"windows": "%APPDATA%\\x", "linux": "~/.local/share/lasrecomp"});
        assert_eq!(resolve_save_folder(&save_folder, Path::new("/library/Game")), Some(PathBuf::from(home).join(".local/share/lasrecomp")));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn save_location_linux_never_falls_back_to_windows_or_android() {
        let game = Path::new("/library/Game");
        assert_eq!(save_location(&json!({"windows": "%APPDATA%\\x", "android": "Android/data/x"}), game), SaveLocation::UnknownOnLinux);
        assert_eq!(save_location(&json!({"android": "/sdcard/x"}), game), SaveLocation::UnknownOnLinux);
        assert_eq!(save_location(&json!({"windows": "saves", "android": "Android/data/x"}), game), SaveLocation::UnknownOnLinux);
        assert_eq!(resolve_save_folder(&json!({"windows": "saves"}), game), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn save_location_linux_empty_values_are_not_listed() {
        let game = Path::new("/library/Game");
        assert_eq!(save_location(&json!({"windows": "", "android": ""}), game), SaveLocation::NotListed);
        assert_eq!(save_location(&json!({"android": ""}), game), SaveLocation::NotListed);
        assert_eq!(save_location(&json!({"windows": "%APPDATA%\\x", "linux": ""}), game), SaveLocation::NotListed);
        assert_eq!(save_location(&json!(""), game), SaveLocation::NotListed);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn save_location_linux_leftover_variable_is_unknown() {
        let game = Path::new("/library/Game");
        assert_eq!(save_location(&json!("%PL_DOES_NOT_EXIST_SAVE%\\osu"), game), SaveLocation::UnknownOnLinux);
        assert_eq!(save_location(&json!({"windows": "x", "linux": "%PL_DOES_NOT_EXIST_SAVE%/x"}), game), SaveLocation::UnknownOnLinux);
        assert_eq!(resolve_save_folder(&json!("%PL_DOES_NOT_EXIST_SAVE%\\osu"), game), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn save_location_linux_key_and_plain_values_still_resolve() {
        let home = std::env::var("HOME").expect("HOME");
        let game = Path::new("/library/Game");
        assert_eq!(save_location(&json!({"windows": "%APPDATA%\\x", "linux": "%HOME%/.local/share/x"}), game), SaveLocation::Known(PathBuf::from(&home).join(".local/share/x")));
        assert_eq!(save_location(&json!("saves"), game), SaveLocation::Known(game.join("saves")));
        assert_eq!(save_location(&json!("100%save"), game), SaveLocation::Known(game.join("100%save")));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn save_location_windows_keeps_upstream_fallbacks() {
        let game = Path::new("C:\\Library\\Game");
        assert_eq!(save_location(&json!({"windows": "saves", "android": "x"}), game), SaveLocation::Known(game.join("saves")));
        assert_eq!(save_location(&json!({"android": "x"}), game), SaveLocation::Known(game.join("x")));
    }

    #[test]
    fn resolve_save_folder_relatif_se_joint_au_dossier_du_jeu() {
        let game_dir = Path::new("C:\\Library\\MyGame");
        let save_folder = Value::String("Save".into());
        assert_eq!(resolve_save_folder(&save_folder, game_dir), Some(game_dir.join("Save")));
    }

    #[test]
    fn resolve_save_folder_avec_variable_reste_absolu() {
        #[cfg(target_os = "windows")]
        let absolute = "C:\\Users\\me\\AppData";
        #[cfg(target_os = "linux")]
        let absolute = "/home/me/.local/share";

        std::env::set_var("PL_TEST_SAVE_VAR", absolute);
        let game_dir = Path::new("C:\\Library\\MyGame");
        #[cfg(target_os = "windows")]
        let save_folder = Value::String("%PL_TEST_SAVE_VAR%\\MyGame".into());
        #[cfg(target_os = "linux")]
        let save_folder = Value::String("%PL_TEST_SAVE_VAR%/MyGame".into());
        let expected = PathBuf::from(absolute).join("MyGame");
        assert_eq!(resolve_save_folder(&save_folder, game_dir), Some(expected));
        std::env::remove_var("PL_TEST_SAVE_VAR");
    }

    #[test]
    fn resolve_preferred_asset_chaine_simple_toute_plateforme() {
        assert_eq!(resolve_preferred_asset(&Value::String("Full.zip".into())), Some("Full.zip".to_string()));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn resolve_preferred_asset_valeur_plate_en_exe_est_ignoree_sous_linux() {
        assert_eq!(resolve_preferred_asset(&Value::String(".exe".into())), None);
        assert_eq!(resolve_preferred_asset(&Value::String("Installer.EXE".into())), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn resolve_preferred_asset_objet_explicite_en_exe_est_honore_sous_linux() {
        assert_eq!(resolve_preferred_asset(&json!({"windows": ".exe", "linux": ".exe"})), Some(".exe".to_string()));
    }

    #[test]
    fn resolve_preferred_asset_par_plateforme() {
        let expected = if get_platform_key() == "windows" { ".exe" } else { ".AppImage" };
        assert_eq!(resolve_preferred_asset(&json!({"windows": ".exe", "linux": ".AppImage"})), Some(expected.to_string()));
    }

    #[test]
    fn resolve_preferred_asset_chaine_vide_est_none() {
        assert_eq!(resolve_preferred_asset(&Value::String(String::new())), None);
    }

    #[test]
    fn resolve_preferred_asset_objet_sans_cle_pour_cette_plateforme_est_none() {
        assert_eq!(resolve_preferred_asset(&json!({"linux": null})), None);
    }
}
