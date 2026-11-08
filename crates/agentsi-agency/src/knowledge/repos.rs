//! The repos of a product. The rules copy `packages/core/src/repos.ts`.

use crate::registry::Product;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoTarget {
    /// The folder, as the registry writes it.
    pub path: String,
    pub scope: Option<String>,
    /// Keeps refs unique when one product spans several repos.
    pub prefix: String,
}

impl RepoTarget {
    pub fn dir(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }

    /// The ref of a file or an issue in this repo.
    pub fn ref_of(&self, rel: &str, sep: char) -> String {
        if self.prefix.is_empty() {
            rel.to_string()
        } else {
            format!("{}{sep}{rel}", self.prefix)
        }
    }
}

pub fn repo_targets(product: &Product) -> Vec<RepoTarget> {
    let many = product.paths.len() > 1;
    product
        .paths
        .iter()
        .filter(|root| Path::new(root).join(".git").exists())
        .map(|root| RepoTarget {
            path: root.clone(),
            scope: None,
            prefix: if many {
                Path::new(root)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                String::new()
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(paths: Vec<String>) -> Product {
        Product {
            id: "p".into(),
            name: "p".into(),
            paths,
            status: "active".into(),
            roles: Vec::new(),
        }
    }

    #[test]
    fn a_product_with_two_repos_prefixes_the_refs() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, plain) = (
            dir.path().join("web"),
            dir.path().join("api"),
            dir.path().join("notes"),
        );
        for d in [&a, &b] {
            std::fs::create_dir_all(d.join(".git")).unwrap();
        }
        std::fs::create_dir_all(&plain).unwrap();
        let paths = [&a, &b, &plain].map(|p| p.to_string_lossy().into_owned());
        let targets = repo_targets(&product(paths.to_vec()));
        assert_eq!(targets.len(), 2, "a folder with no .git is not a repo");
        assert_eq!(targets[0].ref_of("docs/a.md", '/'), "web/docs/a.md");
        assert_eq!(targets[1].ref_of("7", '#'), "api#7");

        let one = repo_targets(&product(vec![paths[0].clone()]));
        assert_eq!(one[0].ref_of("docs/a.md", '/'), "docs/a.md");
    }
}
