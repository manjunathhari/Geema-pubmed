//! Fetches studies by PMID from NCBI efetch and builds `StudyInput` values.

use crate::copied::writer::{NOT_AVAILABLE, StudyInput};
use crate::xml::{Child, Node, XmlError};

/// One record as efetch gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub pmid: String,
    pub title: Option<String>,
    pub journal: Option<String>,
    pub year: Option<i32>,
    pub pub_types: Vec<String>,
    pub doi: Option<String>,
    pub retracted: bool,
    pub abstract_text: Option<String>,
}

const RETRACTED_TYPE: &str = "Retracted Publication";

fn collapse_ws(s: &str) -> String {
    // ASCII whitespace only: thin and no-break spaces in the text are meaningful.
    s.split([' ', '\t', '\n', '\r'])
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn leading_year(s: &str) -> Option<i32> {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
    (digits.len() == 4).then(|| digits.parse().ok()).flatten()
}

fn is_retracted_type(t: &str) -> bool {
    t.trim().eq_ignore_ascii_case(RETRACTED_TYPE)
}

fn clean(n: Option<&Node>) -> Option<String> {
    n.map(|n| collapse_ws(&n.text())).filter(|t| !t.is_empty())
}

fn year_of(date: Option<&Node>) -> Option<i32> {
    let d = date?;
    d.child("Year")
        .or_else(|| d.child("MedlineDate"))
        .and_then(|y| leading_year(&y.text()))
}

fn doi_in(list: Option<&Node>) -> Option<String> {
    list?
        .children("ArticleId")
        .find(|a| a.attr("IdType") == Some("doi"))
        .and_then(|a| clean(Some(a)))
}

fn read_record(top: &Node) -> Option<Record> {
    let cite = top
        .child("MedlineCitation")
        .or_else(|| top.child("BookDocument"))?;
    let pmid = clean(cite.child("PMID"))?;
    let article = cite.child("Article");
    let journal = article.and_then(|a| a.child("Journal"));
    let book = cite.child("Book");

    let title = clean(article.and_then(|a| a.child("ArticleTitle")))
        .or_else(|| clean(article.and_then(|a| a.child("VernacularTitle"))))
        .or_else(|| clean(cite.child("ArticleTitle")))
        .or_else(|| clean(book.and_then(|b| b.child("BookTitle"))));
    let journal_name = clean(journal.and_then(|j| j.child("Title")))
        .or_else(|| clean(journal.and_then(|j| j.child("ISOAbbreviation"))));
    let pub_date = journal
        .and_then(|j| j.child("JournalIssue"))
        .and_then(|i| i.child("PubDate"));
    let year = year_of(pub_date)
        .or_else(|| year_of(article.and_then(|a| a.child("ArticleDate"))))
        .or_else(|| year_of(book.and_then(|b| b.child("PubDate"))));

    // Only the record's own id lists: a reference's list sits deeper.
    let doi = doi_in(
        top.child("PubmedData")
            .and_then(|d| d.child("ArticleIdList")),
    )
    .or_else(|| doi_in(cite.child("ArticleIdList")))
    .or_else(|| {
        article?
            .children("ELocationID")
            .find(|e| e.attr("EIdType") == Some("doi"))
            .and_then(|e| clean(Some(e)))
    });

    let mut pub_types = Vec::new();
    let mut sections: Vec<String> = Vec::new();
    let mut retraction_comment = false;
    top.walk(&mut |n| match n.name.as_str() {
        "PublicationTypeList" | "BookDocument" => {
            pub_types.extend(n.children("PublicationType").filter_map(|t| clean(Some(t))));
        }
        "Abstract" => {
            for t in n.children("AbstractText") {
                if let Some(text) = clean(Some(t)) {
                    sections.push(match t.attr("Label").map(str::trim) {
                        Some(l) if !l.is_empty() => format!("{l}: {text}"),
                        _ => text,
                    });
                }
            }
        }
        "CommentsCorrections" if n.attr("RefType") == Some("RetractionIn") => {
            retraction_comment = true;
        }
        _ => {}
    });
    let retracted = retraction_comment || pub_types.iter().any(|t| is_retracted_type(t));
    let abstract_text = (!sections.is_empty()).then(|| sections.join("\n\n"));
    Some(Record {
        pmid,
        title,
        journal: journal_name,
        year,
        pub_types,
        doi,
        retracted,
        abstract_text,
    })
}

/// Reads an efetch `PubmedArticleSet`. A record with no PMID is skipped.
pub fn parse_efetch(xml: &str) -> Result<Vec<Record>, XmlError> {
    let root = crate::xml::parse(xml)?;
    if root.name != "PubmedArticleSet" {
        return Ok(Vec::new());
    }
    let tops = root.children.iter().filter_map(|c| match c {
        Child::Element(n) if n.name == "PubmedArticle" || n.name == "PubmedBookArticle" => Some(n),
        _ => None,
    });
    Ok(tops.filter_map(read_record).collect())
}

/// The studies for `pmids`, in that order, plus what was missing. Records for
/// any other PMID are ignored. A missing abstract or record keeps the study.
pub fn studies_from(pmids: &[String], records: Vec<Record>) -> Fetched {
    let mut f = Fetched {
        studies: Vec::with_capacity(pmids.len()),
        missing_abstracts: Vec::new(),
        missing_records: Vec::new(),
    };
    for pmid in pmids {
        let r = records.iter().find(|r| &r.pmid == pmid);
        if r.is_none() {
            f.missing_records.push(pmid.clone());
        }
        let abstract_text = r.and_then(|r| r.abstract_text.clone());
        if abstract_text.is_none() {
            f.missing_abstracts.push(pmid.clone());
        }
        f.studies.push(StudyInput {
            pmid: pmid.clone(),
            title: r
                .and_then(|r| r.title.clone())
                .unwrap_or_else(|| "unknown".into()),
            journal: r.and_then(|r| r.journal.clone()),
            year: r.and_then(|r| r.year),
            pub_types: r.map(|r| r.pub_types.clone()).unwrap_or_default(),
            doi: r.and_then(|r| r.doi.clone()),
            retracted: r.is_some_and(|r| r.retracted),
            abstract_text,
            design: NOT_AVAILABLE.into(),
            why: NOT_AVAILABLE.into(),
        });
    }
    f
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub studies: Vec<StudyInput>,
    /// Records PubMed returned without an abstract.
    pub missing_abstracts: Vec<String>,
    /// PMIDs PubMed returned no record for.
    pub missing_records: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = r#"<?xml version="1.0" ?>
<!DOCTYPE PubmedArticleSet PUBLIC "-//NLM//DTD PubMedArticle//EN" "https://dtd.nlm.nih.gov/x.dtd">
<PubmedArticleSet>
<PubmedArticle>
  <MedlineCitation Status="MEDLINE" Owner="NLM">
    <PMID Version="1">11111111</PMID>
    <Article PubModel="Print">
      <Journal>
        <JournalIssue CitedMedium="Internet"><PubDate><Year>2021</Year><Month>Mar</Month></PubDate></JournalIssue>
        <Title>Journal of Made-up
          Surgery</Title>
        <ISOAbbreviation>J Madeup Surg</ISOAbbreviation>
      </Journal>
      <ArticleTitle>A made-up trial of <i>technique</i> A &amp; B.</ArticleTitle>
      <ELocationID EIdType="pii" ValidYN="Y">S0000</ELocationID>
      <ELocationID EIdType="doi" ValidYN="Y">10.0000/location</ELocationID>
      <Abstract>
        <AbstractText Label="BACKGROUND" NlmCategory="BACKGROUND">Made-up background.</AbstractText>
        <AbstractText Label="RESULTS">Pain 4.5 vs 5.5, <i>P</i> &lt; 0.05.</AbstractText>
        <CopyrightInformation>Made-up copyright 1999.</CopyrightInformation>
      </Abstract>
      <PublicationTypeList>
        <PublicationType UI="D1">Journal Article</PublicationType>
        <PublicationType UI="D2">Randomized Controlled Trial</PublicationType>
      </PublicationTypeList>
    </Article>
    <CommentsCorrectionsList>
      <CommentsCorrections RefType="Cites"><PMID Version="1">99999998</PMID></CommentsCorrections>
    </CommentsCorrectionsList>
    <OtherAbstract Type="Publisher"><AbstractText>Should not be read.</AbstractText></OtherAbstract>
  </MedlineCitation>
  <PubmedData>
    <ArticleIdList>
      <ArticleId IdType="pubmed">11111111</ArticleId>
      <ArticleId IdType="doi">10.0000/own.doi</ArticleId>
    </ArticleIdList>
    <ReferenceList><Reference><ArticleIdList>
      <ArticleId IdType="pubmed">99999997</ArticleId>
      <ArticleId IdType="doi">10.0000/someone.else</ArticleId>
    </ArticleIdList></Reference></ReferenceList>
  </PubmedData>
</PubmedArticle>
<PubmedArticle>
  <MedlineCitation>
    <CommentsCorrectionsList>
      <CommentsCorrections RefType="RetractionIn"><PMID Version="1">99999996</PMID></CommentsCorrections>
    </CommentsCorrectionsList>
    <PMID Version="1">22222222</PMID>
    <Article>
      <Journal><JournalIssue><PubDate><MedlineDate>2019 Jan-Feb</MedlineDate></PubDate></JournalIssue>
        <ISOAbbreviation>Madeup Abbrev</ISOAbbreviation></Journal>
      <ArticleTitle>A second made-up record.</ArticleTitle>
      <ELocationID EIdType="doi">10.0000/fallback</ELocationID>
      <PublicationTypeList><PublicationType>Letter</PublicationType></PublicationTypeList>
    </Article>
  </MedlineCitation>
</PubmedArticle>
<PubmedArticle>
  <MedlineCitation><PMID>33333333</PMID>
    <Article>
      <Journal><JournalIssue><PubDate><Season>Spring</Season></PubDate></JournalIssue><Title>J</Title></Journal>
      <ArticleTitle>A third.</ArticleTitle>
      <ArticleDate DateType="Electronic"><Year>2018</Year></ArticleDate>
      <Abstract><AbstractText>Plain abstract.</AbstractText></Abstract>
      <PublicationTypeList><PublicationType> retracted publication </PublicationType></PublicationTypeList>
    </Article>
  </MedlineCitation>
</PubmedArticle>
<PubmedBookArticle>
  <BookDocument><PMID>44444444</PMID>
    <Book><BookTitle>A made-up book</BookTitle><PubDate><Year>2017</Year></PubDate></Book>
    <Abstract><AbstractText>Book abstract.</AbstractText></Abstract>
    <PublicationType>Review</PublicationType>
    <ArticleIdList><ArticleId IdType="doi">10.0000/book</ArticleId></ArticleIdList>
  </BookDocument>
</PubmedBookArticle>
<PubmedArticle><MedlineCitation><Article><ArticleTitle>No PMID, skipped.</ArticleTitle></Article></MedlineCitation></PubmedArticle>
</PubmedArticleSet>"#;

    fn record(pmid: &str) -> Record {
        parse_efetch(ONE)
            .unwrap()
            .into_iter()
            .find(|r| r.pmid == pmid)
            .unwrap()
    }

    #[test]
    fn a_full_record_is_read_field_by_field() {
        let r = record("11111111");
        assert_eq!(
            r.title.as_deref(),
            Some("A made-up trial of technique A & B.")
        );
        assert_eq!(r.journal.as_deref(), Some("Journal of Made-up Surgery"));
        assert_eq!(r.year, Some(2021));
        assert_eq!(
            r.pub_types,
            ["Journal Article", "Randomized Controlled Trial"]
        );
        assert_eq!(
            r.doi.as_deref(),
            Some("10.0000/own.doi"),
            "not a reference's DOI"
        );
        assert!(!r.retracted, "a Cites comment is not a retraction");
        assert_eq!(
            r.abstract_text.as_deref(),
            Some("BACKGROUND: Made-up background.\n\nRESULTS: Pain 4.5 vs 5.5, P < 0.05.")
        );
    }

    #[test]
    fn fallbacks_for_doi_journal_year_and_retraction() {
        let r = record("22222222");
        assert_eq!(r.year, Some(2019), "MedlineDate's leading year");
        assert_eq!(r.journal.as_deref(), Some("Madeup Abbrev"));
        assert_eq!(r.doi.as_deref(), Some("10.0000/fallback"));
        assert!(r.retracted, "RetractionIn comment");
        assert_eq!(r.abstract_text, None);
        let r = record("33333333");
        assert_eq!(r.year, Some(2018), "ArticleDate when PubDate has no year");
        assert!(r.retracted, "publication type, any case");
        assert_eq!(r.abstract_text.as_deref(), Some("Plain abstract."));
    }

    #[test]
    fn a_book_record_is_read() {
        let r = record("44444444");
        assert_eq!(r.title.as_deref(), Some("A made-up book"));
        assert_eq!(r.year, Some(2017));
        assert_eq!(r.doi.as_deref(), Some("10.0000/book"));
        assert_eq!(r.abstract_text.as_deref(), Some("Book abstract."));
        assert_eq!(r.pub_types, ["Review"]);
    }

    #[test]
    fn a_record_without_a_pmid_is_skipped_and_bad_xml_is_an_error() {
        assert_eq!(parse_efetch(ONE).unwrap().len(), 4);
        assert_eq!(parse_efetch("<PubmedArticleSet>"), Err(XmlError));
        assert_eq!(
            parse_efetch("<eFetchResult><ERROR>x</ERROR></eFetchResult>"),
            Ok(vec![])
        );
    }

    #[test]
    fn studies_keep_the_job_order_and_record_what_is_missing() {
        let pmids: Vec<String> = ["33333333", "22222222", "55555555"]
            .map(String::from)
            .to_vec();
        let f = studies_from(&pmids, parse_efetch(ONE).unwrap());
        let got: Vec<&str> = f.studies.iter().map(|s| s.pmid.as_str()).collect();
        assert_eq!(
            got,
            ["33333333", "22222222", "55555555"],
            "only asked PMIDs"
        );
        assert_eq!(f.missing_abstracts, ["22222222", "55555555"]);
        assert_eq!(f.missing_records, ["55555555"]);
        assert_eq!(f.studies[2].title, "unknown");
        assert_eq!(f.studies[0].design, "not available");
        assert_eq!(f.studies[0].why, "not available");
    }
}
