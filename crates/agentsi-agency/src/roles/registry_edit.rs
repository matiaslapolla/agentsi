//! `registry-edit.ts`: a text edit of `agentsi.config.json` that touches one
//! line. The registry keeps one product on each line, so a diff shows one
//! changed line. The owner block is one line too.

use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

static PRODUCT_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(\s*)\{ "id":.*\}(,?)\s*$"#).unwrap());
static OWNER_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(\s*)"owner": \{.*\}(,?)\s*$"#).unwrap());
static LAST_LINE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",?\s*$").unwrap());

/// What `updateProduct` changes in a product.
#[derive(Debug, Clone, Default)]
pub struct Patch {
    pub roles: Option<Vec<String>>,
    /// `Some(None)` removes the `stories` key, `Some(Some(block))` sets it.
    pub stories: Option<Option<Value>>,
    pub name: Option<String>,
    pub paths: Option<Vec<String>>,
}

/// A number as JavaScript prints it: `5.0` is `5`.
fn number(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
        _ => n.to_string(),
    }
}

/// JSON on one line in the registry format: `{ "k": v }` and `["a", "b"]`.
pub fn one_line(v: &Value) -> String {
    match v {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(one_line).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(map) => format!(
            "{{ {} }}",
            map.iter()
                .map(|(k, x)| format!("{}: {}", Value::String(k.clone()), one_line(x)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Number(n) => number(n),
        other => other.to_string(),
    }
}

/// The value as JavaScript holds it: a number such as `5.0` is `5`.
fn canon(v: &Value) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.iter().map(canon).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, x)| (k.clone(), canon(x))).collect())
        }
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or_default();
            if f.fract() == 0.0 && f.abs() < 9e15 {
                Value::from(f as i64)
            } else {
                v.clone()
            }
        }
        other => other.clone(),
    }
}

fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

fn checked(text: String, expected: &Value) -> Result<String, String> {
    if parse(&text)? != *expected {
        return Err("Registry edit did not produce the expected products.".into());
    }
    Ok(text)
}

fn products_mut(doc: &mut Value) -> Result<&mut Vec<Value>, String> {
    doc.get_mut("products")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "The registry has no products.".to_string())
}

/// Rewrites the line of the product with `patch` applied. The other keys keep
/// their order, and every other line stays.
pub fn update_product(text: &str, id: &str, patch: &Patch) -> Result<String, String> {
    let mut doc = parse(text)?;
    let product = products_mut(&mut doc)?
        .iter()
        .find(|p| p.get("id").and_then(Value::as_str) == Some(id))
        .cloned()
        .ok_or_else(|| format!("Unknown product \"{id}\""))?;
    let mut updated: Map<String, Value> = product.as_object().cloned().unwrap_or_default();
    if let Some(name) = &patch.name {
        updated.insert("name".into(), name.clone().into());
    }
    if let Some(paths) = &patch.paths {
        updated.insert("paths".into(), paths.clone().into());
    }
    if let Some(roles) = &patch.roles {
        updated.insert("roles".into(), roles.clone().into());
    }
    match &patch.stories {
        Some(Some(block)) => {
            updated.insert("stories".into(), canon(block));
        }
        Some(None) => {
            updated.shift_remove("stories");
        }
        None => {}
    }
    let updated = Value::Object(updated);

    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    let marker = format!("\"id\": {}", Value::String(id.into()));
    let at = lines.iter().position(|l| l.contains(&marker));
    let caps = at.and_then(|i| PRODUCT_LINE.captures(&lines[i]));
    let (Some(at), Some(caps)) = (at, caps) else {
        return Err(format!("Product \"{id}\" is not one line in the registry."));
    };
    lines[at] = format!("{}{}{}", &caps[1], one_line(&updated), &caps[2]);

    for p in products_mut(&mut doc)? {
        if p.get("id").and_then(Value::as_str) == Some(id) {
            *p = updated.clone();
        }
    }
    checked(lines.join("\n"), &doc)
}

/// Sets `name` and `language` of the owner block, which is one line. The other keys of
/// the block keep their order. A registry with no owner block gets one after `{`.
pub fn update_owner(
    text: &str,
    name: Option<&str>,
    language: Option<&str>,
) -> Result<String, String> {
    let mut doc = parse(text)?;
    let root = doc
        .as_object_mut()
        .ok_or_else(|| "The registry is not an object.".to_string())?;
    let had_owner = root.contains_key("owner");
    let mut owner: Map<String, Value> = match root.get("owner") {
        None => Map::new(),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return Err("The owner block of the registry must be an object.".into()),
    };
    if let Some(name) = name {
        owner.insert("name".into(), name.into());
    }
    if let Some(language) = language {
        owner.insert("language".into(), language.into());
    }
    let owner = Value::Object(owner);
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    if had_owner {
        let at = lines.iter().position(|l| OWNER_LINE.is_match(l));
        let Some(at) = at else {
            return Err("The owner block is not one line in the registry.".into());
        };
        let caps = OWNER_LINE.captures(&lines[at]).expect("the line matched");
        lines[at] = format!("{}\"owner\": {}{}", &caps[1], one_line(&owner), &caps[2]);
        root.insert("owner".into(), owner);
    } else {
        let Some(at) = lines.iter().position(|l| l.trim() == "{") else {
            return Err("The registry has no line with the opening brace alone.".into());
        };
        lines.insert(at + 1, format!("  \"owner\": {},", one_line(&owner)));
        let mut with_owner = Map::new();
        with_owner.insert("owner".into(), owner);
        with_owner.extend(root.clone());
        *root = with_owner;
    }
    checked(lines.join("\n"), &doc)
}

/// Appends a product after the last one, in the one-product-per-line format.
pub fn add_product(text: &str, product: &Value) -> Result<String, String> {
    let product = &canon(product);
    let mut doc = parse(text)?;
    let id = product
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if products_mut(&mut doc)?
        .iter()
        .any(|p| p.get("id").and_then(Value::as_str) == Some(id))
    {
        return Err(format!("Product \"{id}\" is already registered."));
    }
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    let starts_product = |l: &String| {
        let t = l.trim_start();
        t.starts_with("{ \"id\":")
    };
    let Some(last) = lines.iter().rposition(starts_product) else {
        return Err("Registry has no one-line products to append to.".into());
    };
    let indent: String = lines[last]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    lines[last] = LAST_LINE_END.replace(&lines[last], ",").into_owned();
    lines.insert(last + 1, format!("{indent}{}", one_line(product)));
    products_mut(&mut doc)?.push(product.clone());
    checked(lines.join("\n"), &doc)
}

/// Removes the lines of the products in `ids`, each on one line. The other lines stay.
/// A product that is not one line, or an id that the registry lacks, is an error.
pub fn remove_products(text: &str, ids: &[&str]) -> Result<String, String> {
    let mut doc = parse(text)?;
    let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
    for id in ids {
        if !products_mut(&mut doc)?
            .iter()
            .any(|p| p.get("id").and_then(Value::as_str) == Some(id))
        {
            return Err(format!("Unknown product \"{id}\""));
        }
        let marker = format!("\"id\": {}", Value::String((*id).into()));
        let at = lines.iter().position(|l| l.contains(&marker));
        let Some(at) = at.filter(|i| PRODUCT_LINE.is_match(&lines[*i])) else {
            return Err(format!("Product \"{id}\" is not one line in the registry."));
        };
        lines.remove(at);
        products_mut(&mut doc)?.retain(|p| p.get("id").and_then(Value::as_str) != Some(id));
    }
    // The last product line must not end with a comma.
    let last = lines.iter().rposition(|l| PRODUCT_LINE.is_match(l));
    if let Some(last) = last {
        lines[last] = LAST_LINE_END.replace(&lines[last], "").into_owned();
    }
    checked(lines.join("\n"), &doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REGISTRY: &str = r#"{
  "owner": { "name": "Ada" },
  "products": [
    { "id": "a", "name": "A", "paths": ["/x/a"], "status": "active", "roles": ["pm", "dev"] },
    { "id": "b", "name": "B", "paths": ["/x/b"], "status": "planned", "roles": ["pm"] }
  ]
}
"#;

    #[test]
    fn a_roster_edit_changes_one_line() {
        let roles = vec!["pm".to_string(), "qa".to_string()];
        let out = update_product(
            REGISTRY,
            "a",
            &Patch {
                roles: Some(roles),
                ..Patch::default()
            },
        )
        .unwrap();
        let changed: Vec<_> = out
            .lines()
            .zip(REGISTRY.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(
            changed[0].0,
            r#"    { "id": "a", "name": "A", "paths": ["/x/a"], "status": "active", "roles": ["pm", "qa"] },"#
        );
    }

    #[test]
    fn stories_are_set_and_removed_and_a_number_prints_as_javascript_does() {
        let block = json!({"test": "t {files}", "gate": ["g"], "budget_usd": 5.0});
        let out = update_product(
            REGISTRY,
            "b",
            &Patch {
                stories: Some(Some(block)),
                ..Patch::default()
            },
        )
        .unwrap();
        assert!(out.contains(
            r#""roles": ["pm"], "stories": { "test": "t {files}", "gate": ["g"], "budget_usd": 5 } }"#
        ));
        let back = update_product(
            &out,
            "b",
            &Patch {
                stories: Some(None),
                ..Patch::default()
            },
        )
        .unwrap();
        assert_eq!(back, REGISTRY);
    }

    #[test]
    fn a_new_product_goes_after_the_last_one() {
        let product =
            json!({"id": "c", "name": "C", "paths": ["/x/c"], "status": "active", "roles": ["pm"]});
        let out = add_product(REGISTRY, &product).unwrap();
        assert!(out.contains("\"roles\": [\"pm\"] },\n    { \"id\": \"c\""));
        assert_eq!(
            add_product(&out, &product).unwrap_err(),
            "Product \"c\" is already registered."
        );
    }

    #[test]
    fn products_leave_by_their_lines_and_the_last_line_loses_its_comma() {
        let out = remove_products(REGISTRY, &["a"]).unwrap();
        assert!(!out.contains("\"id\": \"a\"") && out.contains("\"id\": \"b\""));
        assert!(out.contains("\"owner\""));
        let none = remove_products(REGISTRY, &["a", "b"]).unwrap();
        assert_eq!(parse(&none).unwrap()["products"], json!([]));
        assert_eq!(
            remove_products(REGISTRY, &["zz"]).unwrap_err(),
            "Unknown product \"zz\""
        );
        let back = add_product(&remove_products(REGISTRY, &["b"]).unwrap(), &json!({"id": "b", "name": "B", "paths": ["/x/b"], "status": "planned", "roles": ["pm"]})).unwrap();
        assert_eq!(back, REGISTRY);
    }

    #[test]
    fn an_unknown_product_and_a_multi_line_product_fail() {
        assert_eq!(
            update_product(REGISTRY, "zz", &Patch::default()).unwrap_err(),
            "Unknown product \"zz\""
        );
        let spread = REGISTRY.replace(
            r#"{ "id": "a", "name": "A","#,
            "{ \"id\": \"a\",\n \"name\": \"A\",",
        );
        assert_eq!(
            update_product(&spread, "a", &Patch::default()).unwrap_err(),
            "Product \"a\" is not one line in the registry."
        );
    }

    #[test]
    fn a_product_gets_a_new_name_and_new_paths_on_its_line() {
        let out = update_product(
            REGISTRY,
            "b",
            &Patch {
                name: Some("Bee".into()),
                paths: Some(vec!["/y/b".into(), "/y/c".into()]),
                ..Patch::default()
            },
        )
        .unwrap();
        assert!(out.contains(
            r#"    { "id": "b", "name": "Bee", "paths": ["/y/b", "/y/c"], "status": "planned", "roles": ["pm"] }"#
        ));
        assert_eq!(out.lines().count(), REGISTRY.lines().count());
    }

    #[test]
    fn the_owner_line_changes_and_keeps_its_other_keys() {
        let text = REGISTRY.replace(r#"{ "name": "Ada" }"#, r#"{ "name": "Ada", "tz": "UTC" }"#);
        let out = update_owner(&text, Some("Bea"), Some("Spanish")).unwrap();
        assert!(
            out.contains(r#"  "owner": { "name": "Bea", "tz": "UTC", "language": "Spanish" },"#)
        );
        assert_eq!(out.lines().count(), text.lines().count());

        let bare = "{\n  \"products\": []\n}\n";
        let added = update_owner(bare, Some("Bea"), None).unwrap();
        assert_eq!(
            added,
            "{\n  \"owner\": { \"name\": \"Bea\" },\n  \"products\": []\n}\n"
        );

        let multi = "{\n  \"owner\": {\n    \"name\": \"Ada\"\n  },\n  \"products\": []\n}\n";
        assert!(update_owner(multi, Some("Bea"), None).is_err());
    }
}
