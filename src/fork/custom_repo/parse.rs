//! Turns what the user typed or pasted into a GitHub project, `owner/project`. The text box accepts what
//! people actually paste: the address bar's link, a link to a sub page of the project (releases, a tag, a
//! folder), a `.git` address, or just `owner/project`. Anything else gets a reason in plain words.

/// A GitHub project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// `owner/project`, the form the catalog uses as a port's key.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }
}

/// Why a link was not taken, each with the sentence the screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Empty,
    /// Not a web address at all (a file path, another kind of link, words).
    NotALink,
    /// A web address, but not GitHub's.
    NotGithub(String),
    /// A person's or organisation's page, with no project in it.
    ProfileOnly,
    /// A GitHub page that is not a project (settings, topics, a search).
    NotAProject,
    /// The owner or project name has characters GitHub does not allow.
    BadName,
}

impl LinkError {
    pub fn text(&self) -> String {
        match self {
            LinkError::Empty => "Type or paste the link to the project's GitHub page first.".to_string(),
            LinkError::NotALink => "That does not look like a web link. It should look like github.com/owner/project.".to_string(),
            LinkError::NotGithub(host) => format!("That link is on {host}. Only GitHub links work here (github.com/owner/project)."),
            LinkError::ProfileOnly => "That is a person's or group's page, not a project. Open the project itself and copy its link: github.com/owner/project.".to_string(),
            LinkError::NotAProject => "That GitHub page is not a project. Open the project itself and copy its link: github.com/owner/project.".to_string(),
            LinkError::BadName => "That owner or project name has characters GitHub does not allow. Copy the link from the project's page.".to_string(),
        }
    }
}

/// GitHub's own pages that sit where an owner name would.
const NOT_OWNERS: &[&str] = &[
    "about", "apps", "collections", "customer-stories", "enterprise", "events", "explore", "features", "issues", "join", "login", "marketplace",
    "new", "notifications", "orgs", "pricing", "pulls", "search", "security", "settings", "sponsors", "team", "topics", "trending",
];

fn valid_owner(s: &str) -> bool {
    !s.is_empty() && s.len() <= 39 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !s.starts_with('-') && !s.ends_with('-')
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Splits `text` into the host and the path after it. `None` when it is not written as an address.
fn split_address(text: &str) -> Result<(String, String), LinkError> {
    // `git@github.com:owner/project.git`
    if let Some(rest) = text.strip_prefix("git@") {
        let (host, path) = rest.split_once(':').ok_or(LinkError::NotALink)?;
        return Ok((host.to_string(), path.to_string()));
    }
    let rest = match text.split_once("://") {
        Some((scheme, rest)) if ["https", "http", "git", "ssh"].iter().any(|s| scheme.eq_ignore_ascii_case(s)) => rest,
        Some(_) => return Err(LinkError::NotALink),
        None => text,
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    // A user name in front of the host (`git@github.com`) is dropped, and so is a port number.
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    Ok((host.to_string(), path.to_string()))
}

/// Reads the text box. Accepts `https://github.com/owner/project` with anything after the project (a releases
/// page, a tag, a folder, a query), `www.`, a trailing slash, a `.git` ending, an SSH-style address, and the
/// short `owner/project`.
pub fn parse_link(input: &str) -> Result<Repo, LinkError> {
    let text = input.trim().trim_matches(|c| matches!(c, '"' | '\'' | '<' | '>' | '`' | '(' | ')')).trim();
    let text = text.split_whitespace().next().unwrap_or("");
    if text.is_empty() {
        return Err(LinkError::Empty);
    }
    let text = text.split(['?', '#']).next().unwrap_or("");
    let first = text.split('/').next().unwrap_or("");
    let written_as_address = text.starts_with("git@") || text.contains("://") || first.contains('.');
    let (host, path) = if written_as_address {
        split_address(text)?
    } else if text.contains('/') && !first.contains(':') {
        // `owner/project`, with GitHub taken as the place.
        ("github.com".to_string(), text.to_string())
    } else {
        return Err(LinkError::NotALink);
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let bare = host.strip_prefix("www.").unwrap_or(&host);
    if bare != "github.com" {
        return Err(if bare.contains('.') { LinkError::NotGithub(bare.to_string()) } else { LinkError::NotALink });
    }
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let owner = parts.next().ok_or(LinkError::ProfileOnly)?;
    let is_github_page = NOT_OWNERS.contains(&owner.to_ascii_lowercase().as_str());
    let Some(name) = parts.next() else {
        return Err(if is_github_page { LinkError::NotAProject } else { LinkError::ProfileOnly });
    };
    if is_github_page {
        return Err(LinkError::NotAProject);
    }
    let name = name.strip_suffix(".git").unwrap_or(name);
    if !valid_owner(owner) || !valid_name(name) {
        return Err(LinkError::BadName);
    }
    Ok(Repo { owner: owner.to_string(), name: name.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(input: &str) -> String {
        parse_link(input).unwrap_or_else(|e| panic!("{input:?} should parse, got {e:?}")).slug()
    }

    #[test]
    fn the_usual_ways_of_writing_a_project_link_all_give_owner_and_project() {
        for input in [
            "github.com/HarbourMasters/Shipwright",
            "https://github.com/HarbourMasters/Shipwright",
            "http://github.com/HarbourMasters/Shipwright",
            "https://www.github.com/HarbourMasters/Shipwright/",
            "HTTPS://GITHUB.COM/HarbourMasters/Shipwright",
            "  https://github.com/HarbourMasters/Shipwright  \n",
            "https://github.com/HarbourMasters/Shipwright.git",
            "git@github.com:HarbourMasters/Shipwright.git",
            "ssh://git@github.com/HarbourMasters/Shipwright.git",
            "HarbourMasters/Shipwright",
            "<https://github.com/HarbourMasters/Shipwright>",
            "\"github.com/HarbourMasters/Shipwright\"",
        ] {
            assert_eq!(ok(input), "HarbourMasters/Shipwright", "{input:?}");
        }
    }

    #[test]
    fn a_link_to_a_page_inside_the_project_still_finds_the_project() {
        for input in [
            "https://github.com/a/b/releases",
            "https://github.com/a/b/releases/tag/v1.2.3",
            "https://github.com/a/b/releases/latest/download/b-linux.zip",
            "https://github.com/a/b/tree/main/docs",
            "https://github.com/a/b/issues/12#issuecomment-5",
            "https://github.com/a/b?tab=readme-ov-file",
            "github.com/a/b/",
        ] {
            assert_eq!(ok(input), "a/b", "{input:?}");
        }
    }

    #[test]
    fn dots_dashes_and_underscores_in_names_are_fine() {
        assert_eq!(ok("github.com/some-one/My_Port.v2"), "some-one/My_Port.v2");
        assert_eq!(ok("github.com/a/b.git.git"), "a/b.git", "only one .git ending is taken off");
    }

    #[test]
    fn the_project_gets_its_web_address_and_catalog_key() {
        let repo = parse_link("git@github.com:a/b.git").unwrap();
        assert_eq!(repo.slug(), "a/b");
        assert_eq!(repo.url(), "https://github.com/a/b");
    }

    #[test]
    fn nothing_typed_says_so() {
        for input in ["", "   ", "\n"] {
            assert_eq!(parse_link(input), Err(LinkError::Empty));
        }
        assert!(LinkError::Empty.text().starts_with("Type or paste"));
    }

    #[test]
    fn a_person_or_group_page_is_not_a_project() {
        for input in ["https://github.com/HarbourMasters", "github.com/HarbourMasters/", "https://github.com"] {
            assert_eq!(parse_link(input), Err(LinkError::ProfileOnly), "{input:?}");
        }
        assert!(LinkError::ProfileOnly.text().contains("Open the project itself"));
    }

    #[test]
    fn githubs_own_pages_are_not_projects() {
        for input in ["https://github.com/topics/zelda", "https://github.com/settings/profile", "https://github.com/orgs/foo/people", "github.com/marketplace", "https://github.com/search?q=zelda"] {
            assert_eq!(parse_link(input), Err(LinkError::NotAProject), "{input:?}");
        }
    }

    #[test]
    fn another_site_is_named_in_the_reason() {
        assert_eq!(parse_link("https://gitlab.com/a/b"), Err(LinkError::NotGithub("gitlab.com".into())));
        assert_eq!(parse_link("https://www.example.org/a/b"), Err(LinkError::NotGithub("example.org".into())));
        assert_eq!(parse_link("https://github.com.evil.example/a/b"), Err(LinkError::NotGithub("github.com.evil.example".into())));
        assert!(LinkError::NotGithub("gitlab.com".into()).text().starts_with("That link is on gitlab.com."));
    }

    #[test]
    fn things_that_are_not_web_links_are_refused() {
        for input in ["zelda", "file:///etc/passwd", "javascript:alert(1)", "C:\\Games\\zelda", "ftp://github.com/a/b", "just some words"] {
            assert_eq!(parse_link(input), Err(LinkError::NotALink), "{input:?}");
        }
    }

    #[test]
    fn names_with_characters_github_does_not_allow_are_refused() {
        for input in ["github.com/a%20b/c", "github.com/-bad/c", "github.com/a/b$c", "github.com/a/..", "github.com/a/."] {
            assert_eq!(parse_link(input), Err(LinkError::BadName), "{input:?}");
        }
        let long = format!("github.com/{}/b", "a".repeat(40));
        assert_eq!(parse_link(&long), Err(LinkError::BadName));
    }

    #[test]
    fn a_pasted_line_with_more_text_after_the_link_takes_the_link() {
        assert_eq!(ok("https://github.com/a/b is the one"), "a/b");
    }

    #[test]
    fn no_reason_names_a_file_or_shows_code() {
        for e in [LinkError::Empty, LinkError::NotALink, LinkError::NotGithub("x.com".into()), LinkError::ProfileOnly, LinkError::NotAProject, LinkError::BadName] {
            let t = e.text().to_lowercase();
            assert!(!t.contains("json") && !t.contains("err") && !t.contains("::"), "{t}");
        }
    }
}
