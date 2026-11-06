//! A very small XML reader for efetch replies. No dependency and no DTD.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Node {
    /// The local name: `mml:math` is `math`.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Child>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Child {
    Element(Node),
    Text(String),
}

/// Anything malformed. Carries no text, so it can't leak input into a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlError;

/// Deeper than any PubMed record; deeper input is treated as malformed.
pub const MAX_DEPTH: usize = 256;

impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The child elements called `name`.
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter_map(move |c| match c {
            Child::Element(n) if n.name == name => Some(n),
            _ => None,
        })
    }

    pub fn child<'a>(&'a self, name: &str) -> Option<&'a Node> {
        self.children.iter().find_map(|c| match c {
            Child::Element(n) if n.name == name => Some(n),
            _ => None,
        })
    }

    /// All text inside this element, inline tags dropped.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.push_text(&mut out);
        out
    }

    fn push_text(&self, out: &mut String) {
        for c in &self.children {
            match c {
                Child::Text(t) => out.push_str(t),
                Child::Element(n) => n.push_text(out),
            }
        }
    }

    /// Calls `f` on this element and every element inside it, in order.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        for c in &self.children {
            if let Child::Element(n) = c {
                n.walk(f);
            }
        }
    }
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

fn decode(raw: &str) -> Result<String, XmlError> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let end = rest.find(';').ok_or(XmlError)?;
        let name = &rest[..end];
        rest = &rest[end + 1..];
        match name {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            "nbsp" => out.push(' '),
            _ => {
                if let Some(num) = name.strip_prefix('#') {
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16),
                        None => num.parse::<u32>(),
                    }
                    .map_err(|_| XmlError)?;
                    out.push(char::from_u32(code).ok_or(XmlError)?);
                } else if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '&')
                {
                    return Err(XmlError);
                } else {
                    // A DTD-defined entity: kept visible, as Incisive does.
                    out.push('&');
                    out.push_str(name);
                    out.push(';');
                }
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// The end of a DOCTYPE starting at `from`: its closing `>`, past any
/// quoted text and internal subset.
fn doctype_end(input: &str, from: usize) -> Result<usize, XmlError> {
    let (mut quote, mut square) = (None, 0usize);
    for (i, c) in input[from..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => square += 1,
            (None, ']') => square = square.saturating_sub(1),
            (None, '>') if square == 0 => return Ok(from + i),
            _ => {}
        }
    }
    Err(XmlError)
}

/// A start tag's body at `from` (just past `<`): the element, whether it
/// closes itself, and the position past its `>`.
fn start_tag(input: &str, from: usize) -> Result<(Node, bool, usize), XmlError> {
    let is_name_end = |c: char| c.is_whitespace() || c == '/' || c == '>' || c == '=';
    let mut rest = &input[from..];
    let name_end = rest.find(is_name_end).ok_or(XmlError)?;
    if name_end == 0 {
        return Err(XmlError);
    }
    let mut node = Node {
        name: local(&rest[..name_end]),
        ..Node::default()
    };
    rest = &rest[name_end..];
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("/>") {
            return Ok((node, true, input.len() - r.len()));
        }
        if let Some(r) = rest.strip_prefix('>') {
            return Ok((node, false, input.len() - r.len()));
        }
        let key_end = rest.find(is_name_end).ok_or(XmlError)?;
        if key_end == 0 {
            return Err(XmlError);
        }
        let key = local(&rest[..key_end]);
        rest = rest[key_end..].trim_start();
        rest = rest.strip_prefix('=').ok_or(XmlError)?.trim_start();
        let quote = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\''))
            .ok_or(XmlError)?;
        rest = &rest[1..];
        let end = rest.find(quote).ok_or(XmlError)?;
        node.attrs.push((key, decode(&rest[..end])?));
        rest = &rest[end + 1..];
    }
}

/// Parses one document. Anything malformed is [`XmlError`].
pub fn parse(input: &str) -> Result<Node, XmlError> {
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let mut i = 0;
    let after = |needle: &str, from: usize| -> Result<(usize, usize), XmlError> {
        let at = input[from..].find(needle).ok_or(XmlError)? + from;
        Ok((at, at + needle.len()))
    };
    while i < input.len() {
        let rest = &input[i..];
        if !rest.starts_with('<') {
            let end = rest.find('<').map_or(input.len(), |n| i + n);
            let raw = &input[i..end];
            match stack.last_mut() {
                Some(top) => top.children.push(Child::Text(decode(raw)?)),
                None if raw.trim().is_empty() => {}
                None => return Err(XmlError),
            }
            i = end;
        } else if rest.starts_with("<?") {
            i = after("?>", i)?.1;
        } else if rest.starts_with("<!--") {
            i = after("-->", i)?.1;
        } else if rest.starts_with("<![CDATA[") {
            let (end, next) = after("]]>", i)?;
            let top = stack.last_mut().ok_or(XmlError)?;
            top.children
                .push(Child::Text(input[i + "<![CDATA[".len()..end].to_string()));
            i = next;
        } else if rest.starts_with("<!") {
            i = doctype_end(input, i)? + 1;
        } else if rest.starts_with("</") {
            let (end, next) = after(">", i)?;
            let name = local(input[i + 2..end].trim());
            let done = stack.pop().ok_or(XmlError)?;
            if done.name != name {
                return Err(XmlError);
            }
            match stack.last_mut() {
                Some(top) => top.children.push(Child::Element(done)),
                None if root.is_none() => root = Some(done),
                None => return Err(XmlError),
            }
            i = next;
        } else {
            let (node, closed, next) = start_tag(input, i + 1)?;
            if closed {
                match stack.last_mut() {
                    Some(top) => top.children.push(Child::Element(node)),
                    None if root.is_none() => root = Some(node),
                    None => return Err(XmlError),
                }
            } else {
                if root.is_some() || stack.len() >= MAX_DEPTH {
                    return Err(XmlError);
                }
                stack.push(node);
            }
            i = next;
        }
    }
    if !stack.is_empty() {
        return Err(XmlError);
    }
    root.ok_or(XmlError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_text_entities_and_cdata_are_read() {
        let n = parse(
            "<?xml version=\"1.0\"?><!DOCTYPE A PUBLIC \"-//x//y\" \"z.dtd\">\n\
             <A k=\"v &amp; w\"><!-- skip --><B>1 &lt; 2 <i>and</i> &#x3E; 0&nbsp;x<![CDATA[ <raw> ]]></B><C/></A>",
        )
        .unwrap();
        assert_eq!(n.name, "A");
        assert_eq!(n.attr("k"), Some("v & w"));
        assert_eq!(n.child("B").unwrap().text(), "1 < 2 and > 0 x <raw> ");
        assert!(n.child("C").is_some());
    }

    #[test]
    fn a_doctype_with_an_internal_subset_is_skipped() {
        let n = parse("<!DOCTYPE A [ <!ENTITY x \"y\"> ]><A>&x;</A>").unwrap();
        assert_eq!(n.text(), "&x;", "an unknown entity stays visible");
    }

    #[test]
    fn namespaced_names_use_the_local_part() {
        let n = parse("<A><mml:math xmlns:mml=\"u\">5</mml:math></A>").unwrap();
        assert_eq!(n.child("math").unwrap().text(), "5");
    }

    #[test]
    fn malformed_input_is_an_error_and_never_panics() {
        for bad in [
            "",
            "text",
            "<A>",
            "<A></B>",
            "</A>",
            "<A><B></A></B>",
            "<A",
            "<A b=>x</A>",
            "<A b=\"1>x</A>",
            "<A>&#xZZ;</A>",
            "<A>&#x110000;</A>",
            "<A/><B/>",
            "<A/>trailing",
            "<!-- open",
            "<A><![CDATA[ open</A>",
            "<é",
            "<A>é&",
        ] {
            assert_eq!(parse(bad), Err(XmlError), "{bad}");
        }
    }

    #[test]
    fn nesting_is_capped_so_the_stack_cannot_overflow() {
        let nested = |d: usize| format!("{}x{}", "<a>".repeat(d), "</a>".repeat(d));
        assert_eq!(parse(&nested(MAX_DEPTH)).unwrap().text(), "x");
        assert_eq!(parse(&nested(200_000)), Err(XmlError));
    }
}
