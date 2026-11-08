//! `voices.toml` (role to `engine:voice`) and `lexicon.toml` (spoken form of words).
//!
//! Same rules as `packages/voice/src/agentsi_voice/config.py` and `text.py`:
//! the file in the data root replaces the default file of `packages/voice/`.

use regex::Regex;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const VOICES: &str = include_str!("../../../../packages/voice/voices.toml");
const LEXICON: &str = include_str!("../../../../packages/voice/lexicon.toml");
const ENGINES: [&str; 3] = ["kokoro", "piper", "chatterbox"];
const DEFAULT_LANG: &str = "en-us";
/// The only engine of the app.
pub const KOKORO: &str = "kokoro";

/// `explicit`, then `AGENTSI_HOME`, then the first line of `$XDG_CONFIG_HOME/agentsi/home`.
/// The order is the one of `agentsi_store::paths::data_root`. The pill does not link the store.
pub fn data_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let expand = |p: &str| -> Option<PathBuf> {
        match p.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                Some(home.clone()?.join(rest.trim_start_matches('/')))
            }
            _ if Path::new(p).is_relative() => Some(home.clone()?.join(p)),
            _ => Some(PathBuf::from(p)),
        }
    };
    if let Some(v) = std::env::var("AGENTSI_HOME")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
    {
        return expand(&v);
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| home.clone().map(|h| h.join(".config")))?;
    let line = std::fs::read_to_string(config.join("agentsi/home")).ok()?;
    let line = line.lines().next()?.trim();
    if line.is_empty() { None } else { expand(line) }
}

/// The text of `<root>/<name>` when that file exists, else `builtin`.
fn read(root: Option<&Path>, name: &str, builtin: &'static str) -> Result<String, String> {
    match root.map(|r| r.join(name)).filter(|p| p.is_file()) {
        Some(path) => {
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
        None => Ok(builtin.to_owned()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceRef {
    pub engine: String,
    pub name: String,
}

impl VoiceRef {
    fn parse(spec: &str) -> Result<VoiceRef, String> {
        match spec.split_once(':') {
            Some((engine, name)) if ENGINES.contains(&engine) && !name.is_empty() => Ok(VoiceRef {
                engine: engine.to_owned(),
                name: name.to_owned(),
            }),
            _ => Err(format!(
                "invalid voice spec \"{spec}\", expected <{}>:<voice>",
                ENGINES.join("|")
            )),
        }
    }

    pub fn id(&self) -> String {
        format!("{}:{}", self.engine, self.name)
    }
}

#[derive(Deserialize)]
struct VoicesFile {
    lang: Option<String>,
    default: String,
    #[serde(default)]
    roles: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voices {
    pub lang: String,
    pub default: VoiceRef,
    pub roles: BTreeMap<String, VoiceRef>,
}

impl Voices {
    pub fn builtin() -> Voices {
        Voices::parse(VOICES).expect("packages/voice/voices.toml is valid, and a test checks it")
    }

    pub fn parse(text: &str) -> Result<Voices, String> {
        let file: VoicesFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let roles = file
            .roles
            .iter()
            .map(|(role, spec)| Ok((role.clone(), VoiceRef::parse(spec)?)))
            .collect::<Result<_, String>>()?;
        Ok(Voices {
            lang: file.lang.unwrap_or_else(|| DEFAULT_LANG.to_owned()),
            default: VoiceRef::parse(&file.default)?,
            roles,
        })
    }

    /// `<root>/voices.toml` if it exists, else the default file.
    pub fn load(root: Option<&Path>) -> Result<Voices, String> {
        Voices::parse(&read(root, "voices.toml", VOICES)?).map_err(|e| format!("voices.toml: {e}"))
    }

    /// The role, then the part of the role before a `.`, then the default.
    pub fn lookup(&self, role: &str) -> &VoiceRef {
        let base = role.split('.').next().unwrap_or(role);
        self.roles
            .get(role)
            .or_else(|| self.roles.get(base))
            .unwrap_or(&self.default)
    }

    /// The Kokoro voice of `role`. A voice of another engine, or one that `known` rejects,
    /// falls back to the default voice, then to the default of the default file.
    /// Each fallback adds one warning.
    pub fn kokoro(&self, role: &str, known: impl Fn(&str) -> bool) -> (String, Vec<String>) {
        let builtin = Voices::builtin().default;
        let mut warnings = Vec::new();
        for (what, voice) in [
            (format!("role \"{role}\""), self.lookup(role)),
            ("the default voice".to_owned(), &self.default),
            ("the built-in default voice".to_owned(), &builtin),
        ] {
            if voice.engine == KOKORO && known(&voice.name) {
                return (voice.name.clone(), warnings);
            }
            warnings.push(format!(
                "{what} uses {}, which the app cannot speak. It falls back.",
                voice.id()
            ));
        }
        (builtin.name, warnings)
    }
}

/// Spoken forms of written words, for one language. Keys are lowercase.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lexicon(HashMap<String, String>);

impl Lexicon {
    /// The section is the language prefix of `lang`: `en` for `en-us`.
    pub fn parse(text: &str, lang: &str) -> Result<Lexicon, String> {
        let file: BTreeMap<String, toml::Value> =
            toml::from_str(text).map_err(|e| e.to_string())?;
        let section = lang.split('-').next().unwrap_or(lang);
        let Some(table) = file.get(section).and_then(toml::Value::as_table) else {
            return Ok(Lexicon::default());
        };
        table
            .iter()
            .map(|(k, v)| match v.as_str() {
                Some(s) => Ok((k.to_lowercase(), s.to_owned())),
                None => Err(format!("[{section}] {k} must be a string")),
            })
            .collect::<Result<_, _>>()
            .map(Lexicon)
    }

    /// `<root>/lexicon.toml` if it exists, else the default file.
    pub fn load(root: Option<&Path>, lang: &str) -> Result<Lexicon, String> {
        Lexicon::parse(&read(root, "lexicon.toml", LEXICON)?, lang)
            .map_err(|e| format!("lexicon.toml: {e}"))
    }

    /// The token with its word replaced by the spoken form. The edge marks stay.
    fn speak(&self, token: &str) -> String {
        let caps = EDGE.captures(token).expect("the pattern matches any token");
        let (prefix, core, suffix) = (&caps[1], &caps[2], &caps[3]);
        match self.0.get(&core.to_lowercase()) {
            Some(spoken) => format!("{prefix}{spoken}{suffix}"),
            None => token.to_owned(),
        }
    }
}

static SENTENCE_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[.!?…]["'»)]*$"#).expect("a fixed pattern"));
static EDGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\W*)(.*?)(\W*)$").expect("a fixed pattern"));

/// Splits on sentence-final punctuation and applies the lexicon, like `text.sentences`.
pub fn sentences(text: &str, lexicon: &Lexicon) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        current.push(lexicon.speak(token));
        if SENTENCE_END.is_match(token) {
            out.push(current.join(" "));
            current.clear();
        }
    }
    if !current.is_empty() {
        out.push(current.join(" "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentsi-voices-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_default_files_parse() {
        let voices = Voices::builtin();
        assert_eq!(voices.lang, "en-us");
        assert_eq!(voices.lookup("pm").id(), "kokoro:af_heart");
        assert_eq!(voices.lookup("dev.backend").id(), "kokoro:am_adam");
        assert_eq!(voices.lookup("nobody").id(), "kokoro:af_heart");
        let lexicon = Lexicon::parse(LEXICON, "en-us").unwrap();
        assert_eq!(
            lexicon.0.get("agentsi").map(String::as_str),
            Some("agent S I")
        );
    }

    #[test]
    fn the_data_root_file_replaces_the_default() {
        let root = temp("replace");
        std::fs::write(
            root.join("voices.toml"),
            "default = \"kokoro:am_puck\"\n[roles]\npm = \"kokoro:bf_emma\"\n",
        )
        .unwrap();
        std::fs::write(root.join("lexicon.toml"), "[en]\nfoo = \"fu\"\n").unwrap();
        let voices = Voices::load(Some(&root)).unwrap();
        assert_eq!(voices.lookup("pm").name, "bf_emma");
        assert_eq!(voices.lookup("dev").name, "am_puck");
        let lexicon = Lexicon::load(Some(&root), &voices.lang).unwrap();
        assert_eq!(sentences("Foo, AgentSI.", &lexicon), vec!["fu, AgentSI."]);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(Voices::load(Some(&root)).unwrap(), Voices::builtin());
    }

    #[test]
    fn another_engine_falls_back_to_the_default_with_a_warning() {
        let voices =
            Voices::parse("default = \"kokoro:af_heart\"\n[roles]\nqa = \"piper:en_US-amy\"\n")
                .unwrap();
        let (name, warnings) = voices.kokoro("qa", |_| true);
        assert_eq!(name, "af_heart");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("piper:en_US-amy"), "{warnings:?}");
        let (name, warnings) = voices.kokoro("pm", |_| true);
        assert_eq!((name.as_str(), warnings.len()), ("af_heart", 0));
    }

    #[test]
    fn an_unknown_voice_and_a_foreign_default_fall_back_to_the_built_in_voice() {
        let voices =
            Voices::parse("default = \"chatterbox:ref\"\n[roles]\npm = \"kokoro:zz_none\"\n")
                .unwrap();
        let (name, warnings) = voices.kokoro("pm", |n| n != "zz_none");
        assert_eq!(name, "af_heart");
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn a_bad_spec_is_an_error() {
        assert!(Voices::parse("default = \"espeak:x\"").is_err());
        assert!(Voices::parse("default = \"kokoro:\"").is_err());
        assert!(Lexicon::parse("[en]\nx = 1\n", "en").is_err());
    }

    #[test]
    fn sentences_split_and_speak_like_the_python_code() {
        let lexicon = Lexicon::parse(LEXICON, "en-us").unwrap();
        assert_eq!(
            sentences("The QA run passed. Ask (AgentSI) now", &lexicon),
            vec!["The Q A run passed.", "Ask (agent S I) now"]
        );
        assert_eq!(
            sentences("He said \"done.\" Then left", &lexicon),
            vec!["He said \"done.\"", "Then left"]
        );
        assert!(sentences("  ", &lexicon).is_empty());
    }

    #[test]
    fn a_missing_section_is_an_empty_lexicon() {
        assert_eq!(Lexicon::parse(LEXICON, "fr").unwrap(), Lexicon::default());
    }
}
