//! Fetching from NCBI, against a mock server. All data is made up.

mod support;

use stage0_runner::pubmed::{NcbiClient, NcbiFailure};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use support::*;

const ONE: &str = "<PubmedArticleSet><PubmedArticle><MedlineCitation><PMID>11111111</PMID><Article>\
<ArticleTitle>Made up</ArticleTitle><Abstract><AbstractText>Made-up abstract.</AbstractText></Abstract>\
</Article></MedlineCitation></PubmedArticle>\
<PubmedArticle><MedlineCitation><PMID>99999999</PMID><Article><ArticleTitle>Never asked for</ArticleTitle>\
</Article></MedlineCitation></PubmedArticle></PubmedArticleSet>";

fn client(m: &Mock) -> NcbiClient {
    let mut c = NcbiClient::new(&format!("{}/entrez/eutils/", m.url));
    c.retry_wait = Duration::from_millis(10);
    c
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_request_holds_only_pmids_and_extra_records_are_ignored() {
    let m = serve(|_, s| respond(s, 200, ONE));
    let f = client(&m).fetch(&ids(&["11111111", "22222222"])).unwrap();
    let got: Vec<&str> = f.studies.iter().map(|s| s.pmid.as_str()).collect();
    assert_eq!(got, ["11111111", "22222222"]);
    assert_eq!(f.studies[0].title, "Made up");
    assert_eq!(f.missing_records, ["22222222"]);
    let r = &m.requests()[0];
    assert_eq!(r.method, "GET");
    assert_eq!(r.body, "", "nothing is sent but the query string");
    assert_eq!(
        r.target,
        "/entrez/eutils/efetch.fcgi?db=pubmed&id=11111111,22222222&rettype=abstract&retmode=xml&tool=stage0-runner"
    );
}

#[test]
fn requests_stay_under_three_a_second() {
    let m = serve(|_, s| respond(s, 200, "<PubmedArticleSet/>"));
    let many: Vec<String> = (0..150).map(|i| format!("{}", 10_000_000 + i)).collect();
    let mut c = client(&m);
    c.fetch(&many).unwrap();
    c.fetch(&many[..1]).unwrap();
    let seen = m.requests();
    assert_eq!(seen.len(), 4, "150 ids at 50 a request, then one more");
    for w in seen.windows(2) {
        let gap = w[1].at - w[0].at;
        // A little under the client's 400 ms: arrival times jitter.
        assert!(gap >= Duration::from_millis(350), "{gap:?}");
    }
    assert_eq!(c.gap, Duration::from_millis(400));
}

#[test]
fn a_server_error_is_retried_then_succeeds() {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = n.clone();
    let m = serve(move |_, s| {
        if n2.fetch_add(1, Ordering::SeqCst) == 0 {
            respond(s, 429, "");
        } else {
            respond(s, 200, ONE);
        }
    });
    let f = client(&m).fetch(&ids(&["11111111"])).unwrap();
    assert_eq!(f.studies[0].title, "Made up");
    assert_eq!(m.requests().len(), 2);
}

#[test]
fn repeated_errors_and_bad_xml_are_failures_with_no_text() {
    let m = serve(|_, s| respond(s, 500, "secret made-up body"));
    assert_eq!(client(&m).fetch(&ids(&["11111111"])), Err(NcbiFailure));
    assert_eq!(m.requests().len(), 3);
    let m = serve(|_, s| respond(s, 200, "<PubmedArticleSet>"));
    assert_eq!(client(&m).fetch(&ids(&["11111111"])), Err(NcbiFailure));
}
