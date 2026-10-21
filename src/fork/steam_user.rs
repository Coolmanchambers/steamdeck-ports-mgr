//! Which Steam account's shortcuts to edit.
//!
//! Steam keeps one `userdata/<account id>/` folder per account that ever signed in, and lists the
//! accounts in `config/loginusers.vdf` (text KeyValues) with a `MostRecent` flag on the one that signed
//! in last. The active account is the one the user saved a choice for, else the `MostRecent` one that
//! still has a `userdata` folder, else the only account there is. With several accounts and no signal
//! the caller has to ask.
//!
//! `loginusers.vdf` keys accounts by 64-bit Steam ID; the folder name is the 32-bit account ID, which
//! is the 64-bit ID minus `STEAM_ID_BASE`. The 64-bit ID never leaves this module.

use super::artwork;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The 64-bit Steam ID of account 0 of an individual user in the public universe.
const STEAM_ID_BASE: u64 = 76_561_197_960_265_728;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamAccount {
    /// The folder name under `userdata/`.
    pub account_id: u32,
    pub account_name: Option<String>,
    pub persona_name: Option<String>,
    pub most_recent: bool,
    pub dir: PathBuf,
}

impl SteamAccount {
    /// What a picker shows: the display name when Steam gave one, else the sign-in name.
    pub fn label(&self) -> String {
        match (self.persona_name.as_deref(), self.account_name.as_deref()) {
            (Some(persona), Some(account)) if persona != account => format!("{persona} ({account})"),
            (Some(name), _) | (None, Some(name)) => name.to_string(),
            (None, None) => format!("Account {}", self.account_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Text(String),
    Map(Vec<(String, Node)>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct TextVdfError(&'static str);

impl std::fmt::Display for TextVdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a readable Steam text file: {}", self.0)
    }
}

impl std::error::Error for TextVdfError {}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Result<Vec<Token>, TextVdfError> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '/' => {
                chars.next();
                if chars.next() != Some('/') {
                    return Err(TextVdfError("stray '/'"));
                }
                while chars.next_if(|&c| c != '\n').is_some() {}
            }
            '{' => {
                chars.next();
                tokens.push(Token::Open);
            }
            '}' => {
                chars.next();
                tokens.push(Token::Close);
            }
            '"' => {
                chars.next();
                let mut value = String::new();
                loop {
                    match chars.next() {
                        None => return Err(TextVdfError("unterminated string")),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some('\\') => value.push('\\'),
                            Some('"') => value.push('"'),
                            Some(other) => {
                                value.push('\\');
                                value.push(other);
                            }
                            None => return Err(TextVdfError("unterminated string")),
                        },
                        Some(other) => value.push(other),
                    }
                }
                tokens.push(Token::Text(value));
            }
            _ => {
                let mut word = String::new();
                while let Some(c) = chars.next_if(|&c| !c.is_whitespace() && !matches!(c, '{' | '}' | '"')) {
                    word.push(c);
                }
                tokens.push(Token::Text(word));
            }
        }
    }
    Ok(tokens)
}

const MAX_TEXT_DEPTH: usize = 16;

fn parse_map(tokens: &mut std::iter::Peekable<std::vec::IntoIter<Token>>, depth: usize, top: bool) -> Result<Vec<(String, Node)>, TextVdfError> {
    if depth > MAX_TEXT_DEPTH {
        return Err(TextVdfError("nested too deeply"));
    }
    let mut entries = Vec::new();
    loop {
        match tokens.next() {
            None if top => return Ok(entries),
            None => return Err(TextVdfError("missing '}'")),
            Some(Token::Close) if !top => return Ok(entries),
            Some(Token::Close) | Some(Token::Open) => return Err(TextVdfError("unexpected brace")),
            Some(Token::Text(key)) => match tokens.next() {
                Some(Token::Text(value)) => entries.push((key, Node::Text(value))),
                Some(Token::Open) => entries.push((key, Node::Map(parse_map(tokens, depth + 1, false)?))),
                _ => return Err(TextVdfError("a key has no value")),
            },
        }
    }
}

fn parse_text_vdf(text: &str) -> Result<Vec<(String, Node)>, TextVdfError> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut tokens = tokenize(text)?.into_iter().peekable();
    parse_map(&mut tokens, 0, true)
}

fn text_of<'a>(fields: &'a [(String, Node)], name: &str) -> Option<&'a str> {
    fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| match v {
        Node::Text(t) => Some(t.as_str()),
        Node::Map(_) => None,
    })
}

/// One entry of `loginusers.vdf`, minus the 64-bit ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginUser {
    pub account_id: u32,
    pub account_name: Option<String>,
    pub persona_name: Option<String>,
    pub most_recent: bool,
    timestamp: u64,
}

/// Reads the accounts out of `loginusers.vdf` text. An entry whose key is not a valid 64-bit Steam ID
/// is skipped; the rest of the file still counts.
pub fn parse_login_users(text: &str) -> Result<Vec<LoginUser>, TextVdfError> {
    let root = parse_text_vdf(text)?;
    let Some((_, Node::Map(users))) = root.iter().find(|(k, _)| k.eq_ignore_ascii_case("users")) else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for (key, node) in users {
        let (Ok(id64), Node::Map(fields)) = (key.parse::<u64>(), node) else { continue };
        let Some(account_id) = id64.checked_sub(STEAM_ID_BASE).and_then(|n| u32::try_from(n).ok()).filter(|&n| n != 0) else { continue };
        let non_empty = |name| text_of(fields, name).filter(|s| !s.is_empty()).map(str::to_string);
        found.push(LoginUser {
            account_id,
            account_name: non_empty("AccountName"),
            persona_name: non_empty("PersonaName"),
            most_recent: text_of(fields, "MostRecent") == Some("1"),
            timestamp: text_of(fields, "Timestamp").and_then(|t| t.parse().ok()).unwrap_or(0),
        });
    }
    Ok(found)
}

/// Every account that has a `userdata/<id>/` folder, enriched from `loginusers.vdf` where it knows
/// the account. Newest sign-in first, so the `MostRecent` account leads. A missing or unreadable
/// `loginusers.vdf` only costs the names and the `MostRecent` flag.
pub fn discover_accounts(steam_root: &Path) -> io::Result<Vec<SteamAccount>> {
    let logins = fs::read_to_string(steam_root.join("config").join("loginusers.vdf"))
        .ok()
        .and_then(|text| parse_login_users(&text).ok())
        .unwrap_or_default();
    let mut accounts: Vec<(u64, SteamAccount)> = artwork::user_dirs(steam_root)?
        .into_iter()
        .filter_map(|dir| {
            let account_id: u32 = dir.file_name()?.to_str()?.parse().ok()?;
            let login = logins.iter().find(|l| l.account_id == account_id);
            let account = SteamAccount {
                account_id,
                account_name: login.and_then(|l| l.account_name.clone()),
                persona_name: login.and_then(|l| l.persona_name.clone()),
                most_recent: login.is_some_and(|l| l.most_recent),
                dir,
            };
            Some((login.map_or(0, |l| l.timestamp), account))
        })
        .collect();
    accounts.sort_by(|(ta, a), (tb, b)| {
        b.most_recent.cmp(&a.most_recent).then(tb.cmp(ta)).then(a.account_id.cmp(&b.account_id))
    });
    Ok(accounts.into_iter().map(|(_, a)| a).collect())
}

/// The account to use: the saved choice when it still exists, else the `MostRecent` one, else the
/// only one. `None` means the caller has to ask.
pub fn pick_active(accounts: &[SteamAccount], saved: Option<u32>) -> Option<&SteamAccount> {
    if let Some(saved) = saved {
        if let Some(found) = accounts.iter().find(|a| a.account_id == saved) {
            return Some(found);
        }
    }
    let mut recent = accounts.iter().filter(|a| a.most_recent);
    match (recent.next(), recent.next()) {
        (Some(only), None) => Some(only),
        (None, _) if accounts.len() == 1 => accounts.first(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGINUSERS: &str = "\"users\"\n{\n\t\"76561198000000001\"\n\t{\n\t\t\"AccountName\"\t\t\"alpha\"\n\t\t\"PersonaName\"\t\t\"Alpha Player\"\n\t\t\"RememberPassword\"\t\t\"1\"\n\t\t\"MostRecent\"\t\t\"0\"\n\t\t\"Timestamp\"\t\t\"1700000000\"\n\t}\n\t\"76561198000000002\"\n\t{\n\t\t\"AccountName\"\t\t\"kid\"\n\t\t\"PersonaName\"\t\t\"kid\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t\t\"Timestamp\"\t\t\"1800000000\"\n\t}\n}\n";

    fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("dpm_steam_user_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn steam_root(name: &str, users: &[u32], loginusers: Option<&str>) -> PathBuf {
        let root = temp_dir(name);
        for id in users {
            fs::create_dir_all(root.join("userdata").join(id.to_string()).join("config")).unwrap();
        }
        fs::create_dir_all(root.join("userdata").join("0")).unwrap();
        if let Some(text) = loginusers {
            fs::create_dir_all(root.join("config")).unwrap();
            fs::write(root.join("config").join("loginusers.vdf"), text).unwrap();
        }
        root
    }

    // 76561198000000001 - 76561197960265728 = 39734273, and so on.
    const ALPHA: u32 = 39_734_273;
    const KID: u32 = 39_734_274;

    #[test]
    fn login_users_are_read_with_the_64_bit_id_turned_into_the_folder_id() {
        let users = parse_login_users(LOGINUSERS).unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!((users[0].account_id, users[0].account_name.as_deref(), users[0].most_recent), (ALPHA, Some("alpha"), false));
        assert_eq!((users[1].account_id, users[1].persona_name.as_deref(), users[1].most_recent), (KID, Some("kid"), true));
    }

    #[test]
    fn entries_with_bad_ids_are_skipped_and_the_rest_still_load() {
        let text = "\"users\" { \"notanumber\" { \"AccountName\" \"x\" } \"5\" { \"AccountName\" \"too small\" } \"76561198000000001\" { \"AccountName\" \"ok\" } }";
        let users = parse_login_users(text).unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].account_name.as_deref(), Some("ok"));
    }

    #[test]
    fn text_syntax_comments_escapes_bom_and_unquoted_words() {
        let text = "\u{FEFF}// comment\n\"users\"\n{\n  76561198000000001 {\n    \"PersonaName\" \"Quote \\\" and \\\\ and \\n\"\n    MostRecent 1 // trailing\n  }\n}\n";
        let users = parse_login_users(text).unwrap();
        assert_eq!(users[0].persona_name.as_deref(), Some("Quote \" and \\ and \n"));
        assert!(users[0].most_recent);
    }

    #[test]
    fn broken_text_is_an_error_not_a_panic() {
        for bad in ["\"users\" {", "\"users\" { \"a\" }", "}", "\"unterminated", "\"users\" { \"k\" \"v\" } }", "/ x"] {
            assert!(parse_login_users(bad).is_err(), "{bad}");
        }
        let deep = "\"a\" {".repeat(40);
        assert!(parse_login_users(&deep).is_err());
    }

    #[test]
    fn a_file_without_a_users_section_has_no_users() {
        assert!(parse_login_users("\"other\" { }").unwrap().is_empty());
        assert!(parse_login_users("").unwrap().is_empty());
    }

    #[test]
    fn accounts_come_from_userdata_and_the_most_recent_one_leads() {
        let root = steam_root("discover", &[ALPHA, KID], Some(LOGINUSERS));
        let accounts = discover_accounts(&root).unwrap();
        assert_eq!(accounts.iter().map(|a| a.account_id).collect::<Vec<_>>(), [KID, ALPHA]);
        assert_eq!(accounts[0].label(), "kid");
        assert_eq!(accounts[1].label(), "Alpha Player (alpha)");
        assert_eq!(accounts[0].dir, root.join("userdata").join(KID.to_string()));
        assert_eq!(pick_active(&accounts, None).unwrap().account_id, KID);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_login_without_a_userdata_folder_is_not_offered() {
        let root = steam_root("nofolder", &[ALPHA], Some(LOGINUSERS));
        let accounts = discover_accounts(&root).unwrap();
        assert_eq!(accounts.len(), 1);
        // The MostRecent account has no folder, so nothing is flagged; the only account wins.
        assert_eq!(pick_active(&accounts, None).unwrap().account_id, ALPHA);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn without_loginusers_accounts_are_nameless_and_a_single_one_is_still_picked() {
        let root = steam_root("nologin", &[ALPHA], None);
        let accounts = discover_accounts(&root).unwrap();
        assert_eq!(accounts[0].label(), format!("Account {ALPHA}"));
        assert_eq!(pick_active(&accounts, None).unwrap().account_id, ALPHA);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn two_accounts_and_no_most_recent_means_ask() {
        let root = steam_root("ask", &[ALPHA, KID], None);
        let accounts = discover_accounts(&root).unwrap();
        assert!(pick_active(&accounts, None).is_none());
        assert_eq!(pick_active(&accounts, Some(ALPHA)).unwrap().account_id, ALPHA);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_saved_choice_beats_most_recent_but_a_stale_one_is_ignored() {
        let root = steam_root("saved", &[ALPHA, KID], Some(LOGINUSERS));
        let accounts = discover_accounts(&root).unwrap();
        assert_eq!(pick_active(&accounts, Some(ALPHA)).unwrap().account_id, ALPHA);
        assert_eq!(pick_active(&accounts, Some(999)).unwrap().account_id, KID);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn two_accounts_flagged_most_recent_means_ask() {
        let both = LOGINUSERS.replace("\"MostRecent\"\t\t\"0\"", "\"MostRecent\"\t\t\"1\"");
        let root = steam_root("both", &[ALPHA, KID], Some(&both));
        let accounts = discover_accounts(&root).unwrap();
        assert!(pick_active(&accounts, None).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_garbled_loginusers_file_falls_back_to_bare_accounts() {
        let root = steam_root("garbled", &[ALPHA], Some("\"users\" {"));
        let accounts = discover_accounts(&root).unwrap();
        assert_eq!(accounts.len(), 1);
        assert!(accounts[0].account_name.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_userdata_folder_is_an_error() {
        let root = temp_dir("nouserdata");
        assert!(discover_accounts(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
