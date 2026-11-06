//! The job file and the program's settings.

use crate::copied::answer::SearchEntry;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub label: String,
    pub question: String,
    pub pmids: Vec<String>,
    /// The stored run's search log. Absent: the prompt says "not available".
    #[serde(default)]
    pub searches: Option<Vec<SearchEntry>>,
}

/// Why the settings or job can't be used. Fixed words only, never file text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    MissingEnv,
    BadOllamaUrl,
    JobUnreadable,
    BadJob,
    OutInsideRepo,
    OutUnusable,
}

/// Labels become file names and log words.
pub fn valid_label(l: &str) -> bool {
    (1..=64).contains(&l.len())
        && l.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_pmid(p: &str) -> bool {
    (1..=9).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit())
}

/// Reads and checks the job text. The parser's error is dropped: it can quote input.
pub fn parse_jobs(text: &[u8]) -> Result<Vec<Job>, ConfigError> {
    let mut jobs: Vec<Job> = serde_json::from_slice(text).map_err(|_| ConfigError::BadJob)?;
    if jobs.is_empty() {
        return Err(ConfigError::BadJob);
    }
    let mut seen: Vec<String> = Vec::new();
    for j in &mut jobs {
        let good = valid_label(&j.label)
            && !seen.contains(&j.label)
            && !j.question.trim().is_empty()
            && !j.pmids.is_empty()
            && j.pmids.iter().all(|p| valid_pmid(p))
            && j.searches.as_ref().is_none_or(|s| !s.is_empty());
        if !good {
            return Err(ConfigError::BadJob);
        }
        seen.push(j.label.clone());
        let mut kept: Vec<String> = Vec::with_capacity(j.pmids.len());
        for p in j.pmids.drain(..) {
            if !kept.contains(&p) {
                kept.push(p);
            }
        }
        j.pmids = kept;
    }
    Ok(jobs)
}

/// Makes the output folder, refusing any place inside a Git repo.
pub fn prepare_out_dir(p: &Path) -> Result<PathBuf, ConfigError> {
    let abs = std::path::absolute(p).map_err(|_| ConfigError::OutUnusable)?;
    let existing = abs
        .ancestors()
        .find(|a| a.exists())
        .ok_or(ConfigError::OutUnusable)?;
    let real = existing
        .canonicalize()
        .map_err(|_| ConfigError::OutUnusable)?;
    if real
        .ancestors()
        .any(|a| a.join(".git").symlink_metadata().is_ok())
    {
        return Err(ConfigError::OutInsideRepo);
    }
    std::fs::create_dir_all(&abs).map_err(|_| ConfigError::OutUnusable)?;
    abs.canonicalize().map_err(|_| ConfigError::OutUnusable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("stage0-job-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn a_good_job_is_read_and_repeated_pmids_go() {
        let j = parse_jobs(
            br#"[{"label":"a-1","question":"Made up?","pmids":["11111111","22222222","11111111"]}]"#,
        )
        .unwrap();
        assert_eq!(j[0].label, "a-1");
        assert_eq!(j[0].pmids, ["11111111", "22222222"]);
    }

    #[test]
    fn a_search_log_is_optional_and_in_the_stored_shape() {
        let j = parse_jobs(br#"[{"label":"a","question":"q","pmids":["1"]}]"#).unwrap();
        assert_eq!(j[0].searches, None);
        let j = parse_jobs(
            br#"[{"label":"a","question":"q","pmids":["1"],
                  "searches":[{"source":"PubMed","query":"made up","hits":3}]}]"#,
        )
        .unwrap();
        let s = j[0].searches.as_ref().unwrap();
        assert_eq!(
            (s[0].source.as_str(), s[0].query.as_str()),
            ("PubMed", "made up")
        );
        assert_eq!(s[0].hits, 3);
    }

    #[test]
    fn bad_jobs_are_refused() {
        for bad in [
            &b"not json"[..],
            b"{}",
            b"[]",
            br#"[{"label":"a","question":"q"}]"#,
            br#"[{"label":"a","question":"q","pmids":[]}]"#,
            br#"[{"label":"a","question":" ","pmids":["1"]}]"#,
            br#"[{"label":"a","question":"q","pmids":["12x"]}]"#,
            br#"[{"label":"a","question":"q","pmids":["1"],"extra":1}]"#,
            br#"[{"label":"../up","question":"q","pmids":["1"]}]"#,
            br#"[{"label":"a b","question":"q","pmids":["1"]}]"#,
            br#"[{"label":"","question":"q","pmids":["1"]}]"#,
            br#"[{"label":"a","question":"q","pmids":["1"]},{"label":"a","question":"r","pmids":["2"]}]"#,
            // An empty log would render as "not available", which means absent.
            br#"[{"label":"a","question":"q","pmids":["1"],"searches":[]}]"#,
            br#"[{"label":"a","question":"q","pmids":["1"],"searches":[{"source":"PubMed","query":"q"}]}]"#,
            br#"[{"label":"a","question":"q","pmids":["1"],"searches":[{"source":"PubMed","query":"q","hits":-1}]}]"#,
        ] {
            assert_eq!(
                parse_jobs(bad),
                Err(ConfigError::BadJob),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn labels_are_safe_as_file_names_and_log_words() {
        assert!(valid_label("run_1-A"));
        assert!(!valid_label(&"x".repeat(65)));
        for bad in ["", "a.b", "a/b", "a b", "é", "a\nb"] {
            assert!(!valid_label(bad), "{bad}");
        }
    }

    #[test]
    fn the_output_folder_is_made_outside_a_repo() {
        let root = temp("ok");
        let out = root.join("deep/out");
        let got = prepare_out_dir(&out).unwrap();
        assert!(got.is_absolute() && got.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_output_folder_can_not_be_inside_a_repo() {
        let root = temp("repo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        for inside in [root.clone(), root.join("out"), root.join("a/b/c")] {
            assert_eq!(prepare_out_dir(&inside), Err(ConfigError::OutInsideRepo));
        }
        assert!(!root.join("out").exists(), "nothing is made in the repo");
        // A `.git` file, as in a worktree, counts too.
        let wt = temp("worktree");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: elsewhere").unwrap();
        assert_eq!(
            prepare_out_dir(&wt.join("o")),
            Err(ConfigError::OutInsideRepo)
        );
        // Through a symlink as well.
        let link = temp("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert_eq!(
            prepare_out_dir(&link.join("o")),
            Err(ConfigError::OutInsideRepo)
        );
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(wt).unwrap();
    }
}
