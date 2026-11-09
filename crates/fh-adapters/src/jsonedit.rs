//! Text-preserving JSON edits. Config files are edited in place: new members and
//! array items are spliced in after the last sibling, and removal cuts exactly the
//! spliced text, so install followed by uninstall restores the original bytes.
//! `//` and `/* */` comments are tolerated (Gemini CLI settings allow them).

use serde_json::Value;

/// Path segment of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seg {
    Key(String),
    Idx(usize),
}

#[derive(Debug)]
pub(crate) enum Kind {
    Obj(Vec<Member>),
    Arr(Vec<Node>),
    Str(String),
    Other,
}

#[derive(Debug)]
pub(crate) struct Node {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

#[derive(Debug)]
pub(crate) struct Member {
    pub key: String,
    pub key_start: usize,
    pub value: Node,
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match &self.kind {
            Kind::Obj(m) => m.iter().find(|m| m.key == key).map(|m| &m.value),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match &self.kind {
            Kind::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn items(&self) -> &[Node] {
        match &self.kind {
            Kind::Arr(a) => a,
            _ => &[],
        }
    }
}

const MAX_DEPTH: usize = 128;

struct P<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

/// Parse JSON text into a span tree.
pub(crate) fn parse(text: &str) -> Result<Node, String> {
    let mut p = P {
        b: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    p.ws()?;
    let n = p.value()?;
    p.ws()?;
    if p.i != p.b.len() {
        return Err(p.err("unexpected text after the top-level value"));
    }
    Ok(n)
}

impl P<'_> {
    fn err(&self, what: &str) -> String {
        let line = self.b[..self.i.min(self.b.len())]
            .iter()
            .filter(|c| **c == b'\n')
            .count()
            + 1;
        format!("{what} at line {line}")
    }

    fn ws(&mut self) -> Result<(), String> {
        loop {
            match self.b.get(self.i) {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.i += 1,
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'/') => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'*') => {
                    let rest = &self.b[self.i + 2..];
                    let end = rest
                        .windows(2)
                        .position(|w| w == b"*/")
                        .ok_or_else(|| self.err("unterminated comment"))?;
                    self.i += end + 4;
                }
                Some(0xEF) if self.i == 0 && self.b.starts_with(&[0xEF, 0xBB, 0xBF]) => self.i = 3,
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self) -> Result<Node, String> {
        let start = self.i;
        match self.b.get(self.i) {
            Some(b'{') => self.container(start, b'}'),
            Some(b'[') => self.container(start, b']'),
            Some(b'"') => {
                let s = self.string()?;
                Ok(Node {
                    start,
                    end: self.i,
                    kind: Kind::Str(s),
                })
            }
            Some(c) if c.is_ascii_alphanumeric() || *c == b'-' => {
                while self
                    .b
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || b"+-.".contains(c))
                {
                    self.i += 1;
                }
                let word = &self.b[start..self.i];
                let ok = matches!(word, b"true" | b"false" | b"null")
                    || serde_json::from_slice::<serde_json::Number>(word).is_ok();
                if !ok {
                    return Err(self.err("invalid literal"));
                }
                Ok(Node {
                    start,
                    end: self.i,
                    kind: Kind::Other,
                })
            }
            _ => Err(self.err("expected a value")),
        }
    }

    fn container(&mut self, start: usize, close: u8) -> Result<Node, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        self.i += 1;
        let mut members = Vec::new();
        let mut items = Vec::new();
        loop {
            self.ws()?;
            if self.b.get(self.i) == Some(&close) {
                self.i += 1;
                break;
            }
            if close == b'}' {
                let key_start = self.i;
                if self.b.get(self.i) != Some(&b'"') {
                    return Err(self.err("expected a member name"));
                }
                let key = self.string()?;
                self.ws()?;
                if self.b.get(self.i) != Some(&b':') {
                    return Err(self.err("expected ':'"));
                }
                self.i += 1;
                self.ws()?;
                let value = self.value()?;
                members.push(Member {
                    key,
                    key_start,
                    value,
                });
            } else {
                items.push(self.value()?);
            }
            self.ws()?;
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(c) if *c == close => {}
                _ => return Err(self.err("expected ',' or the end of the container")),
            }
        }
        self.depth -= 1;
        let kind = if close == b'}' {
            Kind::Obj(members)
        } else {
            Kind::Arr(items)
        };
        Ok(Node {
            start,
            end: self.i,
            kind,
        })
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.i;
        self.i += 1;
        loop {
            match self.b.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => self.i += 2,
                Some(_) => self.i += 1,
            }
        }
        serde_json::from_slice(&self.b[start..self.i]).map_err(|_| self.err("invalid string"))
    }
}

fn nl(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Leading whitespace of the line containing `pos`, if only whitespace precedes `pos`.
fn line_indent(text: &str, pos: usize) -> Option<&str> {
    let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let lead = &text[line_start..pos];
    lead.chars().all(|c| c == ' ' || c == '\t').then_some(lead)
}

fn render(v: &Value, indent: &str, nl: &str) -> String {
    let pretty = serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    pretty.replace('\n', &format!("{nl}{indent}"))
}

/// Splice a child (member text or item) into a container node.
fn insert_child(text: &str, node: &Node, key: Option<&str>, value: &Value) -> String {
    let nl = nl(text);
    let key_txt = |k: &str| format!("{}: ", Value::String(k.to_owned()));
    let last_end_start = match &node.kind {
        Kind::Obj(m) => m.last().map(|m| (m.value.end, m.key_start)),
        Kind::Arr(a) => a.last().map(|n| (n.end, n.start)),
        _ => None,
    };
    let (at_start, at_end, piece) = match last_end_start {
        Some((end, start)) => match line_indent(text, start) {
            Some(ind) => {
                let body = render(value, ind, nl);
                let k = key.map(key_txt).unwrap_or_default();
                (end, end, format!(",{nl}{ind}{k}{body}"))
            }
            None => {
                let k = key.map(key_txt).unwrap_or_default();
                (end, end, format!(", {k}{value}"))
            }
        },
        None => {
            let base = line_indent(text, node.start).unwrap_or("");
            let ind = format!("{base}  ");
            let body = render(value, &ind, nl);
            let k = key.map(key_txt).unwrap_or_default();
            (
                node.start + 1,
                node.end - 1,
                format!("{nl}{ind}{k}{body}{nl}{base}"),
            )
        }
    };
    format!("{}{}{}", &text[..at_start], piece, &text[at_end..])
}

/// Find the node at an object-key path; returns the deepest existing node and how
/// many path keys it consumed.
fn walk<'n>(root: &'n Node, path: &[&str]) -> Result<(&'n Node, usize), String> {
    let mut node = root;
    for (i, k) in path.iter().enumerate() {
        if !matches!(node.kind, Kind::Obj(_)) {
            return Err(format!("\"{}\" is not an object", path[..i].join(".")));
        }
        match node.get(k) {
            Some(n) => node = n,
            None => return Ok((node, i)),
        }
    }
    Ok((node, path.len()))
}

fn nest(path: &[&str], leaf: Value) -> Value {
    path.iter()
        .rev()
        .fold(leaf, |acc, k| serde_json::json!({ (*k): acc }))
}

/// Append `item` to the array at `path`, creating missing objects and the array.
pub(crate) fn append_item(text: &str, path: &[&str], item: &Value) -> Result<String, String> {
    let root = parse(text)?;
    let (node, used) = walk(&root, path)?;
    if used < path.len() {
        let value = nest(&path[used + 1..], Value::Array(vec![item.clone()]));
        return Ok(insert_child(text, node, Some(path[used]), &value));
    }
    if !matches!(node.kind, Kind::Arr(_)) {
        return Err(format!("\"{}\" is not an array", path.join(".")));
    }
    Ok(insert_child(text, node, None, item))
}

/// Add member `key` to the object at `path`, creating missing objects. Leaves the
/// text unchanged when the member already exists.
pub(crate) fn insert_member(
    text: &str,
    path: &[&str],
    key: &str,
    value: &Value,
) -> Result<String, String> {
    let root = parse(text)?;
    let (node, used) = walk(&root, path)?;
    if used < path.len() {
        let mut full: Vec<&str> = path[used + 1..].to_vec();
        full.push(key);
        let v = nest(&full, value.clone());
        return Ok(insert_child(text, node, Some(path[used]), &v));
    }
    if !matches!(node.kind, Kind::Obj(_)) {
        return Err(format!("\"{}\" is not an object", path.join(".")));
    }
    if node.get(key).is_some() {
        return Ok(text.to_owned());
    }
    Ok(insert_child(text, node, Some(key), value))
}

type Pred<'p> = &'p dyn Fn(&[Seg], &Node) -> bool;

/// Remove every node matching `pred`. A container whose children are all removed is
/// removed too (except the root, which becomes empty).
pub(crate) fn remove_where(text: &str, pred: Pred<'_>) -> Result<String, String> {
    let root = parse(text)?;
    let mut ranges = Vec::new();
    collect(&root, &mut Vec::new(), pred, &mut ranges, true);
    ranges.sort_unstable();
    let mut out = text.to_owned();
    for (a, b) in ranges.into_iter().rev() {
        out.replace_range(a..b, "");
    }
    Ok(out)
}

/// Returns whether `node` should be removed entirely; pushes ranges otherwise.
fn collect(
    node: &Node,
    path: &mut Vec<Seg>,
    pred: Pred<'_>,
    out: &mut Vec<(usize, usize)>,
    is_root: bool,
) -> bool {
    if !is_root && pred(path, node) {
        return true;
    }
    // (start, end) of each child, and its own removal ranges.
    let mut spans = Vec::new();
    let mut removed = Vec::new();
    let mut inner = Vec::new();
    let mut visit = |seg: Seg, child: &Node, start: usize, path: &mut Vec<Seg>| {
        path.push(seg);
        let mut r = Vec::new();
        let gone = collect(child, path, pred, &mut r, false);
        path.pop();
        spans.push((start, child.end));
        removed.push(gone);
        inner.push(r);
    };
    match &node.kind {
        Kind::Obj(m) => {
            for mem in m {
                visit(Seg::Key(mem.key.clone()), &mem.value, mem.key_start, path);
            }
        }
        Kind::Arr(a) => {
            for (i, n) in a.iter().enumerate() {
                visit(Seg::Idx(i), n, n.start, path);
            }
        }
        _ => return false,
    }
    if spans.is_empty() || !removed.iter().any(|g| *g) {
        out.extend(inner.into_iter().flatten());
        return false;
    }
    if removed.iter().all(|g| *g) {
        if !is_root {
            return true;
        }
        out.push((node.start + 1, node.end - 1));
        return false;
    }
    let first_kept = removed.iter().position(|g| !g).unwrap_or(0);
    if first_kept > 0 {
        out.push((spans[0].0, spans[first_kept].0));
    }
    for i in first_kept..spans.len() {
        if removed[i] {
            out.push((spans[i - 1].1, spans[i].1));
        } else {
            out.append(&mut inner[i]);
        }
    }
    false
}

/// Whether the text is an object with no members (`{}` with any whitespace).
pub(crate) fn is_empty_object(text: &str) -> bool {
    matches!(parse(text), Ok(Node { kind: Kind::Obj(m), .. }) if m.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn is_ours(p: &[Seg], n: &Node) -> bool {
        crate::plan::is_our_hook(p, n)
    }

    #[test]
    fn round_trips_byte_exact() {
        let originals = [
            "{}\n",
            "{\n  \"model\": \"x\"\n}\n",
            "{\r\n    \"hooks\": {\r\n        \"Stop\": [\r\n            {\"hooks\": [{\"type\": \"command\", \"command\": \"say done\"}]}\r\n        ]\r\n    },\r\n    \"x\": 1\r\n}",
            "{\"a\":1,\"b\":{}}",
            "// comment\n{ /* c */ \"hooks\": { \"Stop\": [] , \"PreToolUse\": [ ] } }",
        ];
        for orig in originals {
            let item = json!({"matcher": "Bash", "hooks": [{"type": "command", "command": "firsthand hook claude-code PreToolUse"}]});
            let a = append_item(orig, &["hooks", "PreToolUse"], &item).unwrap();
            let b = append_item(
                &a,
                &["hooks", "Stop"],
                &json!({"hooks": [{"type": "command", "command": "firsthand hook claude-code Stop"}]}),
            )
            .unwrap();
            let c = insert_member(
                &b,
                &["mcpServers"],
                "firsthand",
                &json!({"command": "firsthand"}),
            )
            .unwrap();
            assert!(parse(&c).is_ok(), "{c}");
            let back = remove_where(&c, &|p: &[Seg], n: &Node| {
                is_ours(p, n) || p == [Seg::Key("mcpServers".into()), Seg::Key("firsthand".into())]
            })
            .unwrap();
            // Pre-existing empty arrays are removed with our only item; all else is exact.
            if orig.contains("[]") || orig.contains("[ ]") {
                assert!(!back.contains("firsthand"), "{back}");
                assert!(back.contains("comment"));
            } else {
                assert_eq!(back, orig);
            }
        }
    }

    #[test]
    fn keeps_user_entries_next_to_ours() {
        let orig = "{\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"hooks\": [\n          {\"type\": \"command\", \"command\": \"firsthand hook claude-code Stop\"},\n          {\"type\": \"command\", \"command\": \"notify-send done\"}\n        ]\n      }\n    ]\n  }\n}\n";
        let out = remove_where(orig, &is_ours).unwrap();
        assert!(out.contains("notify-send") && !out.contains("firsthand"));
        assert!(serde_json::from_str::<Value>(&out).is_ok());
    }

    #[test]
    fn rejects_wrong_shapes() {
        assert!(append_item("{\"hooks\": []}", &["hooks", "Stop"], &json!(1)).is_err());
        assert!(append_item("{\"hooks\": {\"Stop\": {}}}", &["hooks", "Stop"], &json!(1)).is_err());
        assert!(parse("{\"a\": }").is_err());
        assert!(parse(&"[".repeat(500)).is_err());
    }
}
