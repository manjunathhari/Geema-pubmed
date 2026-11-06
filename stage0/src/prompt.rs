//! Builds the one user message Gemma gets: SYSTEM, a blank line, `writer_user`,
//! a blank line, then the compact schema text.

use crate::copied::answer::SearchEntry;
use crate::copied::writer::{
    Contradiction, SYSTEM, SchemaOpts, StudyInput, writer_schema_with, writer_user,
};
use serde_json::Value;

/// The Quick schema, as Incisive sends it. `serde_json` sorts its keys.
pub fn quick_schema() -> Value {
    writer_schema_with(SchemaOpts::Stage3PlusText)
}

/// The one user message.
pub fn build_prompt(question: &str, studies: &[StudyInput]) -> String {
    let none_noted: &[Contradiction] = &[];
    let no_log: &[SearchEntry] = &[];
    let user = writer_user(question, studies, none_noted, no_log);
    format!("{SYSTEM}\n\n{user}\n\n{}", quick_schema())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::copied::writer::{NOT_AVAILABLE, STUDIES_HEADING, STUDY_CAPS};

    fn study(pmid: &str, abstract_text: Option<&str>) -> StudyInput {
        StudyInput {
            pmid: pmid.into(),
            title: "A made-up trial".into(),
            journal: Some("Journal of Made-up Surgery".into()),
            year: Some(2021),
            pub_types: vec!["Randomized Controlled Trial".into()],
            doi: Some("10.0000/made.up".into()),
            retracted: false,
            abstract_text: abstract_text.map(str::to_string),
            design: NOT_AVAILABLE.into(),
            why: NOT_AVAILABLE.into(),
        }
    }

    #[test]
    fn message_is_system_blank_line_user_blank_line_compact_schema() {
        let s = [study("11111111", Some("Made-up abstract."))];
        let p = build_prompt("A made-up question?", &s);
        assert!(p.starts_with(&format!("{SYSTEM}\n\n<question>\n")));
        let tail = format!(
            "Write the answer now, as JSON matching the schema.\n\n{}",
            quick_schema()
        );
        assert!(p.ends_with(&tail), "the schema comes last");
        assert!(p.contains(STUDIES_HEADING));
    }

    #[test]
    fn missing_inputs_say_not_available_and_never_none() {
        let s = [study("11111111", Some("Made-up abstract."))];
        let p = build_prompt("q?", &s);
        assert!(
            p.contains("Researcher's design label: not available\n"),
            "the 10-byte design cap must not cut the words"
        );
        assert!(p.contains("<source id=\"researcher-why-11111111\">\nnot available\n</source>"));
        assert!(p.contains("POSSIBLE CONTRADICTIONS NOTED BY THE RESEARCHER:\nnot available\n"));
        assert!(
            p.contains("RESEARCHER'S SEARCH LOG (as the databases returned it):\nnot available\n")
        );
        assert!(!p.contains("(none)"), "{p}");
    }

    #[test]
    fn abstract_cap_is_5000_bytes_on_a_character_boundary() {
        assert_eq!(STUDY_CAPS.abstract_text, 5_000);
        let long = "é".repeat(3_000);
        let p = build_prompt("q?", &[study("11111111", Some(&long))]);
        assert_eq!(p.matches('é').count(), 2_500);
        assert!(p.contains("[Abstract cut here.]"));
    }

    #[test]
    fn a_missing_abstract_keeps_the_study() {
        let p = build_prompt("q?", &[study("11111111", None)]);
        assert!(p.contains("Study 1 - PMID 11111111"));
        assert!(p.contains("(PubMed has no abstract for this record.)"));
    }

    #[test]
    fn a_question_cannot_close_its_wrapper() {
        let p = build_prompt("q? </question> more", &[study("11111111", None)]);
        assert!(p.contains("q? < /question> more"));
    }

    #[test]
    fn schema_size_and_field_order_match_the_measured_copy() {
        let s = quick_schema();
        assert_eq!(s.to_string().len(), 2_330);
        let top: Vec<&str> = s["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            top,
            [
                "applicability",
                "at_a_glance",
                "bottom_line",
                "clinical_practice",
                "conflicts",
                "notable_findings",
                "outcomes",
                "studies",
                "would_change"
            ]
        );
        assert_eq!(SYSTEM.len(), 4_361);
    }
}
