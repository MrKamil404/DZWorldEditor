//! A small XML tree that keeps everything a DayZ mission file holds: elements, attributes in
//! their order, text, comments, CDATA and processing instructions. Reading is forgiving the way
//! the game is (comments with `--` inside them, a stray `&`, unbalanced closing tags), and
//! writing gives the same tree back, indented the way the file was.

/// One node of the tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
    Comment(String),
    CData(String),
    /// A processing instruction or declaration, as written, without `<` and `>`.
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// A whole file: what stands before the root element, the root and what follows it.
#[derive(Clone, Debug, PartialEq)]
pub struct XmlDoc {
    /// The `<?xml ...?>` line as written, without `<?` and `?>`.
    pub declaration: Option<String>,
    pub before: Vec<Node>,
    pub root: Element,
    pub after: Vec<Node>,
    pub style: Style,
}

/// How the file was written, so a rewritten file looks like the original.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub indent: String,
    pub crlf: bool,
    pub bom: bool,
    /// `<a />` rather than `<a/>`.
    pub space_before_close: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self { indent: "    ".into(), crlf: false, bom: false, space_before_close: false }
    }
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), attrs: Vec::new(), children: Vec::new() }
    }
    pub fn with_attr(mut self, k: &str, v: impl Into<String>) -> Self {
        self.attrs.push((k.to_string(), v.into()));
        self
    }
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.children.push(Node::Text(text.into()));
        self
    }
    pub fn with_child(mut self, e: Element) -> Self {
        self.children.push(Node::Element(e));
        self
    }
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
    /// Attribute by name ignoring case, as the game reads some of them.
    pub fn attr_ci(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
    pub fn attr_or<'a>(&'a self, k: &str, default: &'a str) -> &'a str {
        self.attr(k).unwrap_or(default)
    }
    pub fn set_attr(&mut self, k: &str, v: impl Into<String>) {
        let v = v.into();
        match self.attrs.iter_mut().find(|(n, _)| n == k) {
            Some(a) => {
                if a.1 != v {
                    a.1 = v;
                }
            }
            None => self.attrs.push((k.to_string(), v)),
        }
    }
    pub fn remove_attr(&mut self, k: &str) {
        self.attrs.retain(|(n, _)| n != k);
    }
    pub fn attr_f64(&self, k: &str) -> Option<f64> {
        self.attr(k).and_then(parse_num)
    }
    pub fn attr_f32(&self, k: &str) -> Option<f32> {
        self.attr_f64(k).map(|v| v as f32)
    }
    pub fn attr_i64(&self, k: &str) -> Option<i64> {
        self.attr_f64(k).map(|v| v.round() as i64)
    }
    /// Sets a number, keeping the text as written when it already means that number.
    pub fn set_attr_num(&mut self, k: &str, v: f64) {
        if self.attr_f64(k).is_some_and(|old| same_num(old, v)) {
            return;
        }
        self.set_attr(k, fmt_num(v));
    }
    /// Sets a number given as `f32`, keeping the text when it reads back as the same `f32`.
    pub fn set_attr_f32(&mut self, k: &str, v: f32) {
        if self.attr_f64(k).is_some_and(|old| old as f32 == v) {
            return;
        }
        self.set_attr(k, fmt_f32(v));
    }
    /// Child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }
    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.name == name)
    }
    pub fn children_named_mut<'a>(&'a mut self, name: &'a str) -> impl Iterator<Item = &'a mut Element> + 'a {
        self.elements_mut().filter(move |e| e.name == name)
    }
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }
    pub fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.name == name)
    }
    /// The child of that name, made at the end when missing.
    pub fn ensure_child(&mut self, name: &str) -> &mut Element {
        if self.child(name).is_none() {
            self.children.push(Node::Element(Element::new(name)));
        }
        self.child_mut(name).unwrap()
    }
    /// The child of that name, made before the first later sibling of `order` when missing.
    pub fn ensure_child_ordered(&mut self, name: &str, order: &[&str]) -> &mut Element {
        if self.child(name).is_none() {
            let at = self.insert_position(name, order);
            self.children.insert(at, Node::Element(Element::new(name)));
        }
        self.child_mut(name).unwrap()
    }
    /// Where a new child named `name` belongs among the others, after `order`.
    pub fn insert_position(&self, name: &str, order: &[&str]) -> usize {
        let rank = |n: &str| order.iter().position(|o| *o == n);
        let Some(mine) = rank(name) else { return self.children.len() };
        // After the last child that comes before or with this one, else before the first later one.
        let mut at = None;
        for (i, c) in self.children.iter().enumerate() {
            if let Node::Element(e) = c {
                match rank(&e.name) {
                    Some(r) if r <= mine => at = Some(i + 1),
                    Some(_) if at.is_none() => return i,
                    _ => {}
                }
            }
        }
        at.unwrap_or(self.children.len())
    }
    pub fn push(&mut self, e: Element) {
        self.children.push(Node::Element(e));
    }
    /// Removes the child elements for which `keep` says no.
    pub fn retain_elements(&mut self, mut keep: impl FnMut(&Element) -> bool) {
        self.children.retain(|n| match n {
            Node::Element(e) => keep(e),
            _ => true,
        });
    }
    pub fn remove_children(&mut self, name: &str) {
        self.retain_elements(|e| e.name != name);
    }
    /// All text inside, CDATA included.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for c in &self.children {
            match c {
                Node::Text(t) | Node::CData(t) => s.push_str(t),
                _ => {}
            }
        }
        s
    }
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.text() == text {
            return;
        }
        self.children.retain(|n| !matches!(n, Node::Text(_) | Node::CData(_)));
        if !text.is_empty() {
            self.children.insert(0, Node::Text(text));
        }
    }
    pub fn child_text(&self, name: &str) -> Option<String> {
        self.child(name).map(|e| e.text())
    }
    pub fn child_f64(&self, name: &str) -> Option<f64> {
        self.child(name).and_then(|e| parse_num(e.text().trim()))
    }
    pub fn child_i64(&self, name: &str) -> Option<i64> {
        self.child_f64(name).map(|v| v.round() as i64)
    }
    /// Sets the number in a child element, keeping the text when it already means that number.
    pub fn set_child_num(&mut self, name: &str, v: f64, order: &[&str]) {
        let c = self.ensure_child_ordered(name, order);
        if parse_num(c.text().trim()).is_some_and(|old| same_num(old, v)) {
            return;
        }
        c.set_text(fmt_num(v));
    }
    pub fn set_child_text(&mut self, name: &str, v: &str, order: &[&str]) {
        self.ensure_child_ordered(name, order).set_text(v);
    }
    /// Values of `attr` of the children named `name`, e.g. the usages of a type.
    pub fn child_attrs(&self, name: &str, attr: &str) -> Vec<String> {
        self.children_named(name).filter_map(|e| e.attr(attr)).map(str::to_string).collect()
    }
    /// Makes the children named `name` hold exactly `values` in `attr`. Children that stay keep
    /// their place and other attributes; new ones go after the last of them, or by `order`.
    pub fn set_child_attrs(&mut self, name: &str, attr: &str, values: &[String], order: &[&str]) {
        let now = self.child_attrs(name, attr);
        if now == values {
            return;
        }
        let mut wanted: Vec<&String> = values.iter().collect();
        let mut keep = Vec::new();
        for (i, c) in self.children.iter().enumerate() {
            if let Node::Element(e) = c {
                if e.name == name {
                    let Some(v) = e.attr(attr) else { continue };
                    if let Some(p) = wanted.iter().position(|w| w.as_str() == v) {
                        wanted.remove(p);
                        keep.push(i);
                    }
                }
            }
        }
        let mut i = 0;
        self.children.retain(|n| {
            let stay = match n {
                Node::Element(e) if e.name == name && e.attr(attr).is_some() => keep.contains(&i),
                _ => true,
            };
            i += 1;
            stay
        });
        // Keep the order asked for: rebuild the run of these children in `values` order.
        let positions: Vec<usize> = self.children.iter().enumerate().filter(|(_, n)| matches!(n, Node::Element(e) if e.name == name && e.attr(attr).is_some())).map(|(i, _)| i).collect();
        let at = match positions.last() {
            Some(p) => p + 1,
            None => self.insert_position(name, order),
        };
        for (n, v) in wanted.into_iter().enumerate() {
            self.children.insert(at + n, Node::Element(Element::new(name).with_attr(attr, v.clone())));
        }
        // Sort the run so it follows `values` where the elements are simple.
        let positions: Vec<usize> = self.children.iter().enumerate().filter(|(_, n)| matches!(n, Node::Element(e) if e.name == name && e.attr(attr).is_some())).map(|(i, _)| i).collect();
        let mut run: Vec<Node> = positions.iter().map(|&p| self.children[p].clone()).collect();
        run.sort_by_key(|n| match n {
            Node::Element(e) => values.iter().position(|v| Some(v.as_str()) == e.attr(attr)).unwrap_or(usize::MAX),
            _ => usize::MAX,
        });
        for (p, n) in positions.into_iter().zip(run) {
            self.children[p] = n;
        }
    }
    /// Finds the element at a path of names below this one.
    pub fn path(&self, names: &[&str]) -> Option<&Element> {
        let mut e = self;
        for n in names {
            e = e.child(n)?;
        }
        Some(e)
    }
    pub fn path_mut(&mut self, names: &[&str]) -> Option<&mut Element> {
        let mut e = self;
        for n in names {
            e = e.child_mut(n)?;
        }
        Some(e)
    }
    /// The element at a path of names, made where missing.
    pub fn ensure_path(&mut self, names: &[&str]) -> &mut Element {
        let mut e = self;
        for n in names {
            e = e.ensure_child(n);
        }
        e
    }
    /// The tree with text outside leaf elements trimmed, for comparing content.
    pub fn normalized(&self) -> Element {
        let leaf = !self.children.iter().any(|n| matches!(n, Node::Element(_)));
        let mut out = Element { name: self.name.clone(), attrs: self.attrs.clone(), children: Vec::new() };
        for c in &self.children {
            match c {
                Node::Element(e) => out.children.push(Node::Element(e.normalized())),
                Node::Text(t) if leaf => {
                    if !t.is_empty() {
                        out.children.push(Node::Text(t.clone()))
                    }
                }
                Node::Text(t) => {
                    if !t.trim().is_empty() {
                        out.children.push(Node::Text(t.trim().to_string()))
                    }
                }
                Node::Comment(t) => out.children.push(Node::Comment(t.trim().to_string())),
                other => out.children.push(other.clone()),
            }
        }
        out
    }
}

pub fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok().or_else(|| s.trim_end_matches(['f', 'F']).parse().ok())
}

fn same_num(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1e-300)
}

/// A number as short as it can be written and still read back the same.
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

pub fn fmt_f32(v: f32) -> String {
    if v.fract() == 0.0 && v.abs() < 1e7 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

// ---------------------------------------------------------------- reading

struct Reader<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn starts(&self, p: &str) -> bool {
        self.b[self.i..].starts_with(p.as_bytes())
    }
    fn find(&self, p: &str) -> Option<usize> {
        self.s[self.i..].find(p).map(|n| self.i + n)
    }
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn name(&mut self) -> String {
        let start = self.i;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c.is_ascii_whitespace() || c == b'>' || c == b'/' || c == b'=' || (c == b'?' && self.i > start) {
                break;
            }
            self.i += 1;
        }
        self.s[start..self.i].to_string()
    }
}

/// Turns `&amp;` and friends back into characters; anything else stays as written.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest[..rest.len().min(12)].find(';');
        let decoded = end.and_then(|e| {
            let ent = &rest[1..e];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
                _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

/// Reads a file. Only text that is not XML at all is refused.
pub fn parse(bytes: &[u8]) -> anyhow::Result<XmlDoc> {
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let body = if bom { &bytes[3..] } else { bytes };
    let text = String::from_utf8_lossy(body);
    parse_str_with(&text, bom)
}

pub fn parse_str(text: &str) -> anyhow::Result<XmlDoc> {
    let bom = text.starts_with('\u{feff}');
    parse_str_with(text.trim_start_matches('\u{feff}'), bom)
}

fn parse_str_with(text: &str, bom: bool) -> anyhow::Result<XmlDoc> {
    let mut r = Reader { s: text, b: text.as_bytes(), i: 0 };
    let mut declaration = None;
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut root: Option<Element> = None;
    // Elements open at the moment, innermost last.
    let mut stack: Vec<Element> = Vec::new();
    let mut self_close_space = 0i32;

    fn add(stack: &mut [Element], before: &mut Vec<Node>, after: &mut Vec<Node>, have_root: bool, n: Node) {
        match stack.last_mut() {
            Some(top) => top.children.push(n),
            None if have_root => after.push(n),
            None => before.push(n),
        }
    }

    while r.i < r.b.len() {
        if r.b[r.i] != b'<' {
            let end = r.find("<").unwrap_or(r.b.len());
            let raw = &r.s[r.i..end];
            r.i = end;
            if !stack.is_empty() && !raw.trim().is_empty() {
                stack.last_mut().unwrap().children.push(Node::Text(unescape(raw)));
            } else if !stack.is_empty() {
                // Whitespace between tags; kept for now, dropped from element-holding parents below.
                stack.last_mut().unwrap().children.push(Node::Text(raw.to_string()));
            }
            continue;
        }
        if r.starts("<!--") {
            let start = r.i + 4;
            let end = r.find("-->").unwrap_or(r.b.len());
            let c = r.s[start..end.max(start)].to_string();
            r.i = (end + 3).min(r.b.len());
            add(&mut stack, &mut before, &mut after, root.is_some(), Node::Comment(c));
            continue;
        }
        if r.starts("<![CDATA[") {
            let start = r.i + 9;
            let end = r.find("]]>").unwrap_or(r.b.len());
            let c = r.s[start..end.max(start)].to_string();
            r.i = (end + 3).min(r.b.len());
            add(&mut stack, &mut before, &mut after, root.is_some(), Node::CData(c));
            continue;
        }
        if r.starts("<?") {
            let start = r.i + 2;
            let end = r.find("?>").unwrap_or(r.b.len());
            let c = r.s[start..end.max(start)].to_string();
            r.i = (end + 2).min(r.b.len());
            if c.starts_with("xml") && declaration.is_none() && stack.is_empty() && root.is_none() && before.is_empty() {
                declaration = Some(c);
            } else {
                add(&mut stack, &mut before, &mut after, root.is_some(), Node::Other(format!("?{c}?")));
            }
            continue;
        }
        if r.starts("<!") {
            let start = r.i + 1;
            let end = r.find(">").unwrap_or(r.b.len());
            let c = r.s[start..end.max(start)].to_string();
            r.i = (end + 1).min(r.b.len());
            add(&mut stack, &mut before, &mut after, root.is_some(), Node::Other(c));
            continue;
        }
        if r.starts("</") {
            r.i += 2;
            let name = r.name();
            let end = r.find(">").unwrap_or(r.b.len());
            r.i = (end + 1).min(r.b.len());
            // Close up to the matching element; a closing tag nothing opened is ignored.
            if let Some(depth) = stack.iter().rposition(|e| e.name == name) {
                while stack.len() > depth {
                    let e = finish(stack.pop().unwrap());
                    match stack.last_mut() {
                        Some(top) => top.children.push(Node::Element(e)),
                        None => {
                            if root.is_none() {
                                root = Some(e);
                            } else {
                                after.push(Node::Element(e));
                            }
                        }
                    }
                }
            }
            continue;
        }
        // An opening tag.
        r.i += 1;
        let name = r.name();
        if name.is_empty() {
            // A lone `<`: keep it as text.
            add(&mut stack, &mut before, &mut after, root.is_some(), Node::Text("<".into()));
            continue;
        }
        let mut el = Element::new(name);
        let mut closed = false;
        loop {
            let ws_start = r.i;
            r.skip_ws();
            if r.i >= r.b.len() {
                break;
            }
            match r.b[r.i] {
                b'>' => {
                    r.i += 1;
                    break;
                }
                b'/' => {
                    if r.i > ws_start {
                        self_close_space += 1;
                    } else {
                        self_close_space -= 1;
                    }
                    r.i += 1;
                    r.skip_ws();
                    if r.i < r.b.len() && r.b[r.i] == b'>' {
                        r.i += 1;
                    }
                    closed = true;
                    break;
                }
                _ => {
                    let k = r.name();
                    if k.is_empty() {
                        r.i += 1;
                        continue;
                    }
                    r.skip_ws();
                    let mut v = String::new();
                    if r.i < r.b.len() && r.b[r.i] == b'=' {
                        r.i += 1;
                        r.skip_ws();
                        if r.i < r.b.len() && (r.b[r.i] == b'"' || r.b[r.i] == b'\'') {
                            let q = r.b[r.i] as char;
                            r.i += 1;
                            let end = r.s[r.i..].find(q).map(|n| r.i + n).unwrap_or(r.b.len());
                            v = unescape(&r.s[r.i..end]);
                            r.i = (end + 1).min(r.b.len());
                        } else {
                            let start = r.i;
                            while r.i < r.b.len() && !r.b[r.i].is_ascii_whitespace() && r.b[r.i] != b'>' && !(r.b[r.i] == b'/' && r.b.get(r.i + 1) == Some(&b'>')) {
                                r.i += 1;
                            }
                            v = unescape(&r.s[start..r.i]);
                        }
                    }
                    el.attrs.push((k, v));
                }
            }
        }
        if closed {
            add(&mut stack, &mut before, &mut after, root.is_some(), Node::Element(finish(el)));
            if stack.is_empty() && root.is_none() {
                if let Some(Node::Element(e)) = before.pop() {
                    root = Some(e);
                }
            }
        } else {
            stack.push(el);
        }
    }
    // Elements never closed are closed at the end.
    while let Some(e) = stack.pop() {
        let e = finish(e);
        match stack.last_mut() {
            Some(top) => top.children.push(Node::Element(e)),
            None => {
                if root.is_none() {
                    root = Some(e)
                } else {
                    after.push(Node::Element(e))
                }
            }
        }
    }
    let root = root.ok_or_else(|| anyhow::anyhow!("No XML element in the file"))?;
    before.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
    after.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
    let style = Style { indent: detect_indent(text), crlf: text.contains("\r\n"), bom, space_before_close: self_close_space > 0 };
    Ok(XmlDoc { declaration, before, root, after, style })
}

/// Drops the whitespace between child elements; text of leaf elements stays as it is.
fn finish(mut e: Element) -> Element {
    let has_elements = e.children.iter().any(|n| matches!(n, Node::Element(_) | Node::Comment(_)));
    if has_elements {
        e.children.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
        for n in &mut e.children {
            if let Node::Text(t) = n {
                *t = t.trim().to_string();
            }
        }
    } else {
        // Join the pieces of text the reader made.
        let only_text = e.children.iter().all(|n| matches!(n, Node::Text(_)));
        if only_text && e.children.len() > 1 {
            let t: String = e.children.iter().map(|n| if let Node::Text(t) = n { t.as_str() } else { "" }).collect();
            e.children = vec![Node::Text(t)];
        }
        if e.children.len() == 1 && matches!(&e.children[0], Node::Text(t) if t.trim().is_empty() && t.contains('\n')) {
            e.children.clear();
        }
    }
    e
}

/// The indentation of the first indented line, a tab or some spaces.
fn detect_indent(text: &str) -> String {
    let mut best: Option<String> = None;
    for line in text.lines().take(400) {
        let lead: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if lead.is_empty() || line.trim().is_empty() {
            continue;
        }
        if lead.starts_with('\t') {
            return "\t".into();
        }
        if best.as_ref().is_none_or(|b| lead.len() < b.len()) {
            best = Some(lead);
        }
    }
    best.filter(|b| b.len() <= 8).unwrap_or_else(|| "    ".into())
}

// ---------------------------------------------------------------- writing

impl XmlDoc {
    pub fn new(root: Element) -> Self {
        Self { declaration: Some(r#"xml version="1.0" encoding="UTF-8" standalone="yes""#.into()), before: Vec::new(), root, after: Vec::new(), style: Style::default() }
    }
    pub fn to_string(&self) -> String {
        let nl = if self.style.crlf { "\r\n" } else { "\n" };
        let mut out = String::new();
        if self.style.bom {
            out.push('\u{feff}');
        }
        if let Some(d) = &self.declaration {
            out.push_str("<?");
            out.push_str(d);
            out.push_str("?>");
            out.push_str(nl);
        }
        let w = Writer { style: &self.style, nl };
        for n in &self.before {
            w.node(&mut out, n, 0);
        }
        w.element(&mut out, &self.root, 0);
        for n in &self.after {
            w.node(&mut out, n, 0);
        }
        out
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

struct Writer<'a> {
    style: &'a Style,
    nl: &'a str,
}

impl Writer<'_> {
    fn pad(&self, out: &mut String, depth: usize) {
        for _ in 0..depth {
            out.push_str(&self.style.indent);
        }
    }
    fn node(&self, out: &mut String, n: &Node, depth: usize) {
        match n {
            Node::Element(e) => self.element(out, e, depth),
            Node::Text(t) => {
                self.pad(out, depth);
                out.push_str(&escape_text(t));
                out.push_str(self.nl);
            }
            Node::Comment(c) => {
                // Multi-line comments keep their own layout.
                if !c.contains('\n') {
                    self.pad(out, depth);
                }
                out.push_str("<!--");
                out.push_str(&if self.style.crlf { c.replace("\r\n", "\n").replace('\n', "\r\n") } else { c.replace("\r\n", "\n") });
                out.push_str("-->");
                out.push_str(self.nl);
            }
            Node::CData(c) => {
                self.pad(out, depth);
                out.push_str("<![CDATA[");
                out.push_str(c);
                out.push_str("]]>");
                out.push_str(self.nl);
            }
            Node::Other(o) => {
                self.pad(out, depth);
                out.push('<');
                out.push_str(o);
                out.push('>');
                out.push_str(self.nl);
            }
        }
    }
    fn open(&self, out: &mut String, e: &Element) {
        out.push('<');
        out.push_str(&e.name);
        for (k, v) in &e.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape_attr(v));
            out.push('"');
        }
    }
    fn element(&self, out: &mut String, e: &Element, depth: usize) {
        self.pad(out, depth);
        self.open(out, e);
        if e.children.is_empty() {
            out.push_str(if self.style.space_before_close { " />" } else { "/>" });
            out.push_str(self.nl);
            return;
        }
        let inline = e.children.iter().all(|n| matches!(n, Node::Text(_) | Node::CData(_)));
        out.push('>');
        if inline {
            for n in &e.children {
                match n {
                    Node::Text(t) => out.push_str(&escape_text(t)),
                    Node::CData(c) => {
                        out.push_str("<![CDATA[");
                        out.push_str(c);
                        out.push_str("]]>");
                    }
                    _ => {}
                }
            }
        } else {
            out.push_str(self.nl);
            for n in &e.children {
                self.node(out, n, depth + 1);
            }
            self.pad(out, depth);
        }
        out.push_str("</");
        out.push_str(&e.name);
        out.push('>');
        out.push_str(self.nl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_the_same_tree() {
        let src = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<types>\r\n\t<!-- weapons -->\r\n\t<type name=\"AK&amp;M\">\r\n\t\t<nominal>10</nominal>\r\n\t\t<flags count_in_map=\"1\" />\r\n\t\t<usage name=\"Military\"/>\r\n\t</type>\r\n</types>\r\n";
        let d = parse(src.as_bytes()).unwrap();
        assert_eq!(d.style.indent, "\t");
        assert!(d.style.crlf);
        let t = d.root.child("type").unwrap();
        assert_eq!(t.attr("name"), Some("AK&M"));
        assert_eq!(t.child_i64("nominal"), Some(10));
        let out = d.to_string();
        let again = parse(out.as_bytes()).unwrap();
        assert_eq!(again.root, d.root);
        assert_eq!(again.declaration, d.declaration);
        assert!(out.contains("AK&amp;M"));
    }

    #[test]
    fn forgives_what_the_game_forgives() {
        let src = "<a>\n<!-- ------ bad -- comment ------ -->\n<b x='1' y=2/>\n<c>tom & jerry</c>\n</x>\n<d></d></a>";
        let d = parse(src.as_bytes()).unwrap();
        assert_eq!(d.root.name, "a");
        assert!(matches!(&d.root.children[0], Node::Comment(c) if c.contains("bad -- comment")));
        let b = d.root.child("b").unwrap();
        assert_eq!(b.attr("x"), Some("1"));
        assert_eq!(b.attr("y"), Some("2"));
        assert_eq!(d.root.child_text("c").unwrap(), "tom & jerry");
        assert!(d.root.child("d").is_some());
    }

    #[test]
    fn numbers_keep_their_text_when_unchanged() {
        let mut e = Element::new("pos").with_attr("x", "12.50").with_attr("z", "4422.178223");
        e.set_attr_num("x", 12.5);
        e.set_attr_f32("z", "4422.178223".parse::<f64>().unwrap() as f32);
        assert_eq!(e.attr("x"), Some("12.50"));
        assert_eq!(e.attr("z"), Some("4422.178223"));
        e.set_attr_num("x", 13.0);
        assert_eq!(e.attr("x"), Some("13"));
    }

    #[test]
    fn child_lists_keep_order_and_extras() {
        let mut t = Element::new("type").with_child(Element::new("nominal").with_text("1")).with_child(Element::new("usage").with_attr("name", "Town")).with_child(Element::new("value").with_attr("name", "Tier1"));
        t.set_child_attrs("usage", "name", &["Town".into(), "Farm".into()], &["nominal", "usage", "value"]);
        let names: Vec<_> = t.elements().map(|e| (e.name.clone(), e.attr("name").map(str::to_string))).collect();
        assert_eq!(names[1], ("usage".into(), Some("Town".into())));
        assert_eq!(names[2], ("usage".into(), Some("Farm".into())));
        assert_eq!(names[3].0, "value");
        t.set_child_attrs("category", "name", &["tools".into()], &["nominal", "category", "usage", "value"]);
        assert_eq!(t.elements().nth(1).unwrap().name, "category");
        t.set_child_num("lifetime", 30.0, &["nominal", "lifetime", "category"]);
        assert_eq!(t.elements().nth(1).unwrap().name, "lifetime");
    }
}
