//! The guide of `docs/guide/README.md`. The program holds the page and its images, so an
//! installed app with no checkout shows it. The Guide screen draws it.

use std::ops::Range;
use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat, ImageSource, SharedUri};

/// The page, in Markdown.
pub const PAGE: &str = include_str!("../../../docs/guide/README.md");

/// The anchor of the section "Install and start".
pub const INSTALL: &str = "install-and-start";

/// The images of the page, by the link that the page uses.
macro_rules! images {
    ($($name:literal),* $(,)?) => {
        [$((
            concat!("img/", $name, ".png"),
            include_bytes!(concat!("../../../docs/guide/img/", $name, ".png")) as &[u8],
        )),*]
    };
}

const IMAGES: [(&str, &[u8]); 18] = images![
    "call",
    "front-desk",
    "git",
    "meeting",
    "memory",
    "minute",
    "minutes-list",
    "palette",
    "priorities",
    "roles",
    "settings-data",
    "settings-dictation",
    "settings-keys",
    "settings-voice",
    "settings",
    "stories",
    "story",
    "window-tree",
];

/// The embedded image of the link `uri`, or none. The same link gives the same `Image`, so
/// the window decodes each PNG once.
pub fn image(uri: &str) -> Option<Arc<Image>> {
    static DECODED: OnceLock<Vec<(&str, Arc<Image>)>> = OnceLock::new();
    DECODED
        .get_or_init(|| {
            IMAGES
                .iter()
                .map(|(link, bytes)| {
                    (
                        *link,
                        Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec())),
                    )
                })
                .collect()
        })
        .iter()
        .find(|(link, _)| *link == uri)
        .map(|(_, image)| image.clone())
}

/// The source of an image of the page. A link that names no embedded image stays a link, and
/// the page shows no image for it.
pub fn image_source(uri: &SharedUri) -> ImageSource {
    match image(uri.as_ref()) {
        Some(image) => image.into(),
        None => uri.clone().into(),
    }
}

/// A heading of the page.
#[derive(Debug, PartialEq, Eq)]
pub struct Heading {
    /// 1 for `#`, 2 for `##`, and so on.
    pub level: usize,
    pub title: String,
    /// The anchor that GitHub gives to the heading.
    pub anchor: String,
    /// The bytes of the title in the page.
    pub source: Range<usize>,
}

/// The anchor of a heading as GitHub makes it: lower case, spaces as hyphens, and no other
/// punctuation.
pub fn anchor_of(title: &str) -> String {
    title
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// The headings of `page`, in order. A line in a code block is not a heading. A second heading
/// with the same anchor gets `-1`, a third `-2`, as on GitHub.
pub fn headings(page: &str) -> Vec<Heading> {
    let mut found: Vec<Heading> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut fenced = false;
    let mut at = 0;
    for line in page.split_inclusive('\n') {
        let start = at;
        at += line.len();
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let hashes = line.bytes().take_while(|&b| b == b'#').count();
        if fenced || !(1..=6).contains(&hashes) || line.as_bytes().get(hashes) != Some(&b' ') {
            continue;
        }
        let title = line[hashes + 1..].trim_end();
        let base = anchor_of(title);
        let seen = seen.entry(base.clone()).or_insert(0);
        let anchor = if *seen == 0 {
            base
        } else {
            format!("{base}-{seen}")
        };
        *seen += 1;
        found.push(Heading {
            level: hashes,
            title: title.to_string(),
            anchor,
            source: start + hashes + 1..start + hashes + 1 + title.len(),
        });
    }
    found
}

/// The bytes of the title of the section `anchor` in the page.
pub fn section(anchor: &str) -> Option<Range<usize>> {
    headings(PAGE)
        .into_iter()
        .find(|h| h.anchor == anchor)
        .map(|h| h.source)
}

/// What a click on a link of the page does.
#[derive(Debug, PartialEq, Eq)]
pub enum Link<'a> {
    /// Opens the browser.
    Browser(&'a str),
    /// Scrolls the page to the section with this anchor.
    Section(&'a str),
    /// Nothing: the link names a file of the repo, which an installed app does not hold.
    Nothing,
}

pub fn link(url: &str) -> Link<'_> {
    if url.starts_with("https://") || url.starts_with("http://") {
        Link::Browser(url)
    } else if let Some(anchor) = url.strip_prefix('#') {
        Link::Section(anchor)
    } else {
        Link::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder of the guide in the checkout.
    fn guide_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guide")
    }

    /// The text of `page` that a reader sees: no code block, no code span, no link target.
    fn prose(page: &str) -> String {
        let mut text = String::new();
        let mut fenced = false;
        for line in page.lines() {
            if line.starts_with("```") {
                fenced = !fenced;
            } else if !fenced {
                text.push_str(line);
                text.push('\n');
            }
        }
        let mut out = String::new();
        let mut code = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '`' => code = !code,
                ']' if !code && chars.peek() == Some(&'(') => {
                    // Skip the target of a link or an image.
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                c if !code => out.push(c),
                _ => {}
            }
        }
        out
    }

    /// Each target `(...)` of a link or an image of the page, with its text.
    fn targets(page: &str) -> Vec<(String, String)> {
        let mut found = Vec::new();
        let mut fenced = false;
        for line in page.lines() {
            if line.starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if fenced {
                continue;
            }
            let mut rest = line;
            while let Some(at) = rest.find("](") {
                let before = &rest[..at];
                let text = before.rsplit_once('[').map_or("", |(_, t)| t);
                let after = &rest[at + 2..];
                let Some(end) = after.find(')') else { break };
                found.push((text.to_string(), after[..end].to_string()));
                rest = &after[end..];
            }
        }
        found
    }

    #[test]
    fn the_page_holds_each_section_heading_of_the_old_page_with_its_anchor() {
        let found = headings(PAGE);
        let expected = [
            (2, "What AgentSI is", "what-agentsi-is"),
            (3, "The pieces", "the-pieces"),
            (2, "Install and start", "install-and-start"),
            (3, "What you need", "what-you-need"),
            (3, "Steps", "steps"),
            (3, "The Claude Code plugin", "the-claude-code-plugin"),
            (2, "The window", "the-window"),
            (3, "The sidebar tree", "the-sidebar-tree"),
            (3, "Tabs", "tabs"),
            (3, "Context menus", "context-menus"),
            (3, "The command palette", "the-command-palette"),
            (3, "The screens", "the-screens"),
            (3, "Settings and the look", "settings-and-the-look"),
            (3, "The Git screen", "the-git-screen"),
            (2, "Products", "products"),
            (3, "The registry", "the-registry"),
            (
                3,
                "Find the product of a folder",
                "find-the-product-of-a-folder",
            ),
            (3, "Add a product", "add-a-product"),
            (
                3,
                "What AgentSI touches in your repo",
                "what-agentsi-touches-in-your-repo",
            ),
            (2, "Meetings", "meetings"),
            (3, "Start a meeting", "start-a-meeting"),
            (3, "Read the minutes", "read-the-minutes"),
            (3, "Listen to the call", "listen-to-the-call"),
            (2, "Decisions and priorities", "decisions-and-priorities"),
            (3, "The desk", "the-desk"),
            (3, "Mark the priorities", "mark-the-priorities"),
            (3, "From the terminal", "from-the-terminal"),
            (3, "Archive", "archive"),
            (2, "Stories", "stories"),
            (3, "The steps", "the-steps"),
            (2, "Roles learn", "roles-learn"),
            (3, "Reflect", "reflect"),
            (3, "Memory", "memory"),
            (3, "Brief, roster and audit", "brief-roster-and-audit"),
            (2, "Voice", "voice"),
            (3, "Download the models", "download-the-models"),
            (3, "Dictation", "dictation"),
            (3, "Conversation mode", "conversation-mode"),
            (3, "Voice mode", "voice-mode"),
            (3, "The vocabulary", "the-vocabulary"),
            (2, "Keys", "keys"),
            (3, "The dictation key", "the-dictation-key"),
            (2, "Data", "data"),
            (3, "Backups", "backups"),
            (3, "Clear data", "clear-data"),
            (
                2,
                "The CLI and the Claude Code plugin",
                "the-cli-and-the-claude-code-plugin",
            ),
            (3, "The CLI", "the-cli"),
            (3, "The Claude Code plugin", "the-claude-code-plugin-1"),
        ];
        let got: Vec<(usize, &str, &str)> = found
            .iter()
            .filter(|h| h.level > 1)
            .map(|h| (h.level, h.title.as_str(), h.anchor.as_str()))
            .collect();
        assert_eq!(got, expected);
        assert_eq!(found.iter().filter(|h| h.level == 1).count(), 1);
    }

    #[test]
    fn the_page_holds_each_image_of_the_old_page_with_its_alt_text() {
        let images: Vec<(String, String)> = targets(PAGE)
            .into_iter()
            .filter(|(_, target)| target.starts_with("img/"))
            .collect();
        assert_eq!(images.len(), 18);
        let mut seen = std::collections::BTreeSet::new();
        for (alt, target) in &images {
            assert!(seen.insert(target.clone()), "{target} is twice in the page");
            assert!(
                PAGE.contains(&format!("![{alt}]({target})")),
                "{target} is not an image"
            );
            assert!(alt.len() > 40, "{target} has a short alt text");
            assert!(guide_dir().join(target).is_file(), "{target} is no file");
        }
        // Each PNG file of the folder is in the page or in the README of the repo.
        let readme = std::fs::read_to_string(guide_dir().join("../../README.md")).unwrap();
        for file in std::fs::read_dir(guide_dir().join("img")).unwrap() {
            let name = file.unwrap().file_name().into_string().unwrap();
            assert!(
                PAGE.contains(&format!("(img/{name})"))
                    || readme.contains(&format!("docs/guide/img/{name}")),
                "img/{name} is in no page"
            );
        }
    }

    #[test]
    fn each_image_link_of_the_page_resolves_to_an_embedded_image() {
        for (_, target) in targets(PAGE).iter().filter(|(_, t)| t.starts_with("img/")) {
            let image = image(target).unwrap_or_else(|| panic!("{target} is not embedded"));
            assert_eq!(image.format, ImageFormat::Png);
            let file = std::fs::read(guide_dir().join(target)).unwrap();
            assert_eq!(
                image.bytes.as_slice(),
                file.as_slice(),
                "{target} differs from its file"
            );
            let again = super::image(target).unwrap();
            assert!(Arc::ptr_eq(&image, &again), "{target} decodes twice");
        }
        assert!(image("img/none.png").is_none());
        assert!(image("https://example.com/call.png").is_none());
    }

    #[test]
    fn each_link_of_the_page_resolves_to_a_file_or_a_heading() {
        let anchors: Vec<String> = headings(PAGE).into_iter().map(|h| h.anchor).collect();
        let links = targets(PAGE);
        assert!(links.len() > 30, "{} links", links.len());
        for (_, target) in links {
            assert!(!target.contains("://"), "{target} leaves the repo");
            let (path, fragment) = target.split_once('#').unwrap_or((&target, ""));
            if path.is_empty() {
                assert!(
                    anchors.contains(&fragment.to_string()),
                    "no heading {fragment}"
                );
            } else {
                assert!(guide_dir().join(path).exists(), "{target} is not a file");
            }
        }
    }

    #[test]
    fn the_contents_list_links_each_section() {
        let sections: Vec<Heading> = headings(PAGE)
            .into_iter()
            .filter(|h| h.level == 2)
            .collect();
        assert_eq!(sections.len(), 12);
        for section in sections {
            assert!(
                PAGE.contains(&format!("]({}{})", "#", section.anchor)),
                "no link to {}",
                section.anchor
            );
        }
    }

    #[test]
    fn the_page_is_english_and_names_no_web_screen_postgres_or_make_dev() {
        assert!(!PAGE.contains("<html"), "the page is Markdown");
        let page = PAGE.to_lowercase();
        for banned in [
            "postgres",
            "make dev",
            "make guide",
            "3361",
            "127.0.0.1",
            "como-funciona",
            "pnpm dev",
            "web ui",
            "web app",
            "localhost",
        ] {
            // The registry example says "a web app" as a kind of product.
            let hits = page.matches(banned).count();
            let allowed = usize::from(banned == "web app");
            assert!(hits <= allowed, "the guide says \"{banned}\"");
        }
    }

    #[test]
    fn the_prose_of_the_page_has_no_semicolon() {
        let mut text = prose(PAGE);
        // `&lt;` and its kind end with a semicolon that is part of the entity.
        for entity in ["&lt;", "&gt;", "&amp;"] {
            text = text.replace(entity, "");
        }
        assert!(!text.contains(';'), "a semicolon in the prose");
    }

    #[test]
    fn the_page_draws_no_html_that_the_app_cannot_draw() {
        for tag in ["<kbd", "<svg", "<img", "<figure", "<div", "<details"] {
            assert!(!PAGE.contains(tag), "the page has {tag}");
        }
    }

    #[test]
    fn the_page_has_the_section_that_the_palette_opens() {
        let range = section(INSTALL).expect("the section exists");
        assert_eq!(&PAGE[range], "Install and start");
        assert_eq!(section("no-such-section"), None);
    }

    #[test]
    fn a_heading_in_a_code_block_is_no_heading_and_a_twin_gets_a_number() {
        let page =
            "# A\n\n```sh\n# not a heading\n```\n\n## B c!\n\n## B c!\n\n## B c!\n\n#nospace\n";
        let found = headings(page);
        let anchors: Vec<&str> = found.iter().map(|h| h.anchor.as_str()).collect();
        assert_eq!(anchors, ["a", "b-c", "b-c-1", "b-c-2"]);
        assert_eq!(&page[found[1].source.clone()], "B c!");
    }

    #[test]
    fn a_link_opens_the_browser_scrolls_the_page_or_does_nothing() {
        assert_eq!(
            link("https://example.com/a"),
            Link::Browser("https://example.com/a")
        );
        assert_eq!(
            link("http://example.com"),
            Link::Browser("http://example.com")
        );
        assert_eq!(link("#products"), Link::Section("products"));
        assert_eq!(link("../../CONTRIBUTING.md"), Link::Nothing);
        assert_eq!(
            link("../../CONTRIBUTING.md#dictation-key-hyprland"),
            Link::Nothing
        );
        assert_eq!(link("file:///etc/passwd"), Link::Nothing);
        assert_eq!(link("javascript:alert(1)"), Link::Nothing);
    }
}
