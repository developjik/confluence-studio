//! Confluence storage XHTML <-> Markdown conversion (contract subset).
//!
//! Best-effort bidirectional conversion. Perfect round-trips are not guaranteed,
//! but headings, list and body text must survive a round trip.

use quick_xml::Reader;
use quick_xml::events::Event;

// ---------------------------------------------------------------------------
// XML node tree
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Node {
    Elem {
        name: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    },
    Text(String),
}

/// Open element on the parser stack: (name, attrs, children so far).
type Frame = (String, Vec<(String, String)>, Vec<Node>);

fn parse_nodes(storage: &str) -> Vec<Node> {
    let mut reader = Reader::from_str(storage);
    let mut roots: Vec<Node> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    fn push_node(stack: &mut [Frame], roots: &mut Vec<Node>, node: Node) {
        match stack.last_mut() {
            Some((_, _, children)) => children.push(node),
            None => roots.push(node),
        }
    }

    fn attrs_of(e: &quick_xml::events::BytesStart) -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        for a in e.attributes().flatten() {
            attrs.push((
                String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                a.unescape_value()
                    .map(|v| v.into_owned())
                    .unwrap_or_default(),
            ));
        }
        attrs
    }

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                stack.push((name, attrs_of(&e), Vec::new()));
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let node = Node::Elem {
                    name,
                    attrs: attrs_of(&e),
                    children: Vec::new(),
                };
                push_node(&mut stack, &mut roots, node);
            }
            Ok(Event::End(_)) => {
                if let Some((name, attrs, children)) = stack.pop() {
                    let node = Node::Elem {
                        name,
                        attrs,
                        children,
                    };
                    push_node(&mut stack, &mut roots, node);
                }
            }
            Ok(Event::Text(t)) => {
                let s = t.unescape().map(|c| c.into_owned()).unwrap_or_default();
                push_node(&mut stack, &mut roots, Node::Text(s));
            }
            Ok(Event::CData(t)) => {
                let s = String::from_utf8_lossy(t.as_ref()).into_owned();
                push_node(&mut stack, &mut roots, Node::Text(s));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break, // best effort: keep whatever parsed so far
        }
    }
    roots
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Concatenated text content (no whitespace normalization).
fn raw_text(nodes: &[Node]) -> String {
    let mut s = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => s.push_str(t),
            Node::Elem { children, .. } => s.push_str(&raw_text(children)),
        }
    }
    s
}

/// Collapse whitespace runs to a single space, keeping boundary spaces.
fn normalize_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

fn escape_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn attr_escape(s: &str) -> String {
    html_escape(s).replace('"', "&quot;")
}

fn longest_backtick_run(s: &str) -> usize {
    let mut max = 0;
    let mut cur = 0;
    for c in s.chars() {
        if c == '`' {
            cur += 1;
            if cur > max {
                max = cur;
            }
        } else {
            cur = 0;
        }
    }
    max
}

fn code_span(t: &str) -> String {
    if t.contains('`') {
        format!("`` {} ``", t)
    } else {
        format!("`{}`", t)
    }
}

// ---------------------------------------------------------------------------
// storage -> markdown
// ---------------------------------------------------------------------------

const TRANSPARENT_BLOCKS: &[&str] = &[
    "div",
    "section",
    "article",
    "figure",
    "ac:layout",
    "ac:layout-section",
    "ac:layout-cell",
    "ac:rich-text-body",
];

fn is_block_name(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "pre"
            | "blockquote"
            | "table"
            | "hr"
            | "ac:structured-macro"
            | "ac:image"
    ) || TRANSPARENT_BLOCKS.contains(&name)
}

/// Convert Confluence storage XHTML to Markdown.
pub fn storage_to_markdown(storage: &str) -> String {
    let nodes = parse_nodes(storage);
    let blocks = blocks_from_nodes(&nodes);
    if blocks.is_empty() {
        return String::new();
    }
    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

fn blocks_from_nodes(nodes: &[Node]) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut inline_run: Vec<&Node> = Vec::new();

    for n in nodes {
        match n {
            Node::Text(_) => inline_run.push(n),
            Node::Elem {
                name,
                attrs,
                children,
            } => {
                if is_block_name(name) {
                    flush_inline(&mut inline_run, &mut blocks);
                    blocks.extend(block_md(name, attrs, children));
                } else {
                    inline_run.push(n);
                }
            }
        }
    }
    flush_inline(&mut inline_run, &mut blocks);
    blocks
}

fn flush_inline(run: &mut Vec<&Node>, blocks: &mut Vec<String>) {
    if run.is_empty() {
        return;
    }
    let md = inline_storage(run).trim().to_string();
    run.clear();
    if !md.is_empty() {
        blocks.push(md);
    }
}

fn block_md(name: &str, attrs: &[(String, String)], children: &[Node]) -> Vec<String> {
    match name {
        "p" => {
            let s = inline_storage(&children.iter().collect::<Vec<_>>())
                .trim()
                .to_string();
            if s.is_empty() { vec![] } else { vec![s] }
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name[1..].parse::<usize>().unwrap_or(1);
            let s = inline_storage(&children.iter().collect::<Vec<_>>())
                .trim()
                .to_string();
            if s.is_empty() {
                vec![]
            } else {
                vec![format!("{} {}", "#".repeat(level), s)]
            }
        }
        "pre" => vec![pre_md(attrs, children)],
        "blockquote" => {
            let inner = blocks_from_nodes(children).join("\n\n");
            vec![quote_md(&inner)]
        }
        "ul" => vec![list_md(children, false, 0)],
        "ol" => vec![list_md(children, true, 0)],
        "table" => {
            let t = table_md(children);
            if t.is_empty() { vec![] } else { vec![t] }
        }
        "hr" => vec!["---".to_string()],
        "ac:structured-macro" => vec![macro_md(attrs, children)],
        "ac:image" => {
            let f = attachment_filename(children).unwrap_or_default();
            vec![format!("![]({})", f)]
        }
        _ if TRANSPARENT_BLOCKS.contains(&name) => blocks_from_nodes(children),
        _ => vec![],
    }
}

fn pre_md(attrs: &[(String, String)], children: &[Node]) -> String {
    let lang = attr(attrs, "data-lang")
        .map(str::to_string)
        .or_else(|| {
            attr(attrs, "class")
                .and_then(|c| c.strip_prefix("language-"))
                .map(str::to_string)
        })
        .unwrap_or_default();
    let content = raw_text(children).trim_end_matches('\n').to_string();
    let fence = "`".repeat(std::cmp::max(3, longest_backtick_run(&content) + 1));
    format!("{}{}\n{}\n{}", fence, lang, content, fence)
}

fn quote_md(inner: &str) -> String {
    inner
        .lines()
        .map(|l| {
            if l.trim().is_empty() {
                ">".to_string()
            } else {
                format!("> {}", l)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn macro_md(attrs: &[(String, String)], children: &[Node]) -> String {
    let name = attr(attrs, "ac:name").unwrap_or("macro").to_uppercase();
    let body = blocks_from_nodes(children).join("\n\n");
    let mut out = format!("> [!{}]", name);
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&quote_md(&body));
    }
    out
}

fn list_md(children: &[Node], ordered: bool, depth: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut n = 0usize;
    for c in children {
        let (cname, cchildren) = match c {
            Node::Elem { name, children, .. } => (name.as_str(), children.as_slice()),
            Node::Text(_) => continue,
        };
        if cname != "li" {
            continue;
        }
        n += 1;
        let mut inline_nodes: Vec<&Node> = Vec::new();
        let mut nested: Vec<(&str, &[Node])> = Vec::new();
        for cc in cchildren {
            match cc {
                Node::Text(_) => inline_nodes.push(cc),
                Node::Elem { name, children, .. } => match name.as_str() {
                    "ul" | "ol" => nested.push((name.as_str(), children.as_slice())),
                    "p" | "div" => inline_nodes.extend(children.iter()),
                    "ac:parameter" => {}
                    _ => inline_nodes.push(cc),
                },
            }
        }
        let inline = inline_storage(&inline_nodes).trim().to_string();
        let marker = if ordered {
            format!("{}. ", n)
        } else {
            "- ".to_string()
        };
        lines.push(format!("{}{}{}", " ".repeat(depth * 4), marker, inline));
        for (tag, kids) in nested {
            lines.push(list_md(kids, tag == "ol", depth + 1));
        }
    }
    lines.join("\n")
}

fn attachment_filename(nodes: &[Node]) -> Option<String> {
    nodes.iter().find_map(|n| match n {
        Node::Elem { name, attrs, .. } if name == "ri:attachment" => {
            attr(attrs, "ri:filename").map(str::to_string)
        }
        _ => None,
    })
}

fn link_text(nodes: &[Node]) -> String {
    for n in nodes {
        if let Node::Elem { name, children, .. } = n
            && name == "ac:plain-text-link-body"
        {
            let t = raw_text(children).trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    for n in nodes {
        if let Node::Elem { name, attrs, .. } = n
            && name.starts_with("ri:")
            && let Some(t) = attr(attrs, "ri:content-title")
        {
            return t.to_string();
        }
    }
    raw_text(nodes).trim().to_string()
}

fn table_md(children: &[Node]) -> String {
    let mut rows: Vec<(Vec<String>, bool)> = Vec::new();

    fn collect_rows(nodes: &[Node], in_thead: Option<bool>, rows: &mut Vec<(Vec<String>, bool)>) {
        for n in nodes {
            if let Node::Elem { name, children, .. } = n {
                match name.as_str() {
                    "thead" => collect_rows(children, Some(true), rows),
                    "tbody" | "tfoot" => collect_rows(children, Some(false), rows),
                    "tr" => {
                        let mut cells: Vec<String> = Vec::new();
                        let mut header = in_thead == Some(true);
                        for c in children {
                            if let Node::Elem { name, children, .. } = c {
                                match name.as_str() {
                                    "th" | "td" => {
                                        if name == "th" {
                                            header = true;
                                        }
                                        let cell =
                                            inline_storage(&children.iter().collect::<Vec<_>>())
                                                .trim()
                                                .replace('|', "\\|");
                                        cells.push(cell);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if !cells.is_empty() {
                            rows.push((cells, header));
                        }
                    }
                    _ => collect_rows(children, in_thead, rows),
                }
            }
        }
    }

    collect_rows(children, None, &mut rows);
    if rows.is_empty() {
        return String::new();
    }

    let header_idx = rows.iter().position(|(_, h)| *h).unwrap_or(0);
    let (header, _) = rows.remove(header_idx);
    let width = std::cmp::max(
        header.len(),
        rows.iter().map(|(c, _)| c.len()).max().unwrap_or(0),
    );

    let fmt_row = |cells: &[String]| -> String {
        let mut cs = cells.to_vec();
        cs.resize(width, String::new());
        format!("| {} |", cs.join(" | "))
    };

    let mut out = fmt_row(&header);
    out.push('\n');
    out.push_str(&format!("|{}|", vec![" --- "; width].join("|")));
    for (cells, _) in &rows {
        out.push('\n');
        out.push_str(&fmt_row(cells));
    }
    out
}

fn inline_storage(nodes: &[&Node]) -> String {
    let mut out = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(&escape_md(&normalize_ws(t))),
            Node::Elem {
                name,
                attrs,
                children,
            } => {
                let kids: Vec<&Node> = children.iter().collect();
                match name.as_str() {
                    "strong" | "b" => {
                        let inner = inline_storage(&kids).trim().to_string();
                        if !inner.is_empty() {
                            out.push_str(&format!("**{}**", inner));
                        }
                    }
                    "em" | "i" => {
                        let inner = inline_storage(&kids).trim().to_string();
                        if !inner.is_empty() {
                            out.push_str(&format!("*{}*", inner));
                        }
                    }
                    "code" => {
                        let raw = raw_text(children).trim().to_string();
                        if !raw.is_empty() {
                            out.push_str(&code_span(&raw));
                        }
                    }
                    "a" => {
                        let inner = inline_storage(&kids).trim().to_string();
                        match attr(attrs, "href") {
                            Some(h) if !h.is_empty() && !inner.is_empty() => {
                                out.push_str(&format!("[{}]({})", inner, h))
                            }
                            _ => out.push_str(&inner),
                        }
                    }
                    "br" => out.push_str("  \n"),
                    "img" => {
                        let src = attr(attrs, "src").unwrap_or("");
                        let alt = attr(attrs, "alt").unwrap_or("");
                        out.push_str(&format!("![{}]({})", alt, src));
                    }
                    "ac:link" => out.push_str(&link_text(children)),
                    "ac:image" => {
                        let f = attachment_filename(children).unwrap_or_default();
                        out.push_str(&format!("![]({})", f));
                    }
                    "ac:parameter" => {}
                    _ => out.push_str(&inline_storage(&kids)),
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// markdown -> storage
// ---------------------------------------------------------------------------

/// Convert Markdown to Confluence storage XHTML.
pub fn markdown_to_storage(md: &str) -> String {
    let normalized = md.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut blocks: Vec<String> = Vec::new();
    let mut para: Vec<String> = Vec::new();

    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if let Some(lang) = fence_open(trimmed) {
            flush_para(&mut para, &mut blocks);
            let (block, next) = parse_fence(&lines, i, &lang);
            blocks.push(block);
            i = next;
            continue;
        }
        if trimmed.is_empty() {
            flush_para(&mut para, &mut blocks);
            i += 1;
            continue;
        }
        if let Some((level, text)) = heading_parts(trimmed) {
            flush_para(&mut para, &mut blocks);
            blocks.push(format!("<h{lvl}>{}</h{lvl}>", inline_md(text), lvl = level));
            i += 1;
            continue;
        }
        if is_hr(trimmed) {
            flush_para(&mut para, &mut blocks);
            blocks.push("<hr/>".to_string());
            i += 1;
            continue;
        }
        if trimmed.starts_with('>') {
            flush_para(&mut para, &mut blocks);
            let (block, next) = parse_blockquote(&lines, i);
            blocks.push(block);
            i = next;
            continue;
        }
        if is_table_start(&lines, i) {
            flush_para(&mut para, &mut blocks);
            let (block, next) = parse_table(&lines, i);
            blocks.push(block);
            i = next;
            continue;
        }
        if list_marker(trimmed).is_some() {
            flush_para(&mut para, &mut blocks);
            let (block, next) = parse_list_block(&lines, i);
            blocks.push(block);
            i = next;
            continue;
        }
        para.push(line.to_string());
        i += 1;
    }
    flush_para(&mut para, &mut blocks);
    blocks.join("\n")
}

fn flush_para(para: &mut Vec<String>, blocks: &mut Vec<String>) {
    if para.is_empty() {
        return;
    }
    // (rendered line, hard line break after it)
    let parts: Vec<(String, bool)> = para
        .drain(..)
        .map(|line| {
            let hard_break = line.ends_with("  ");
            (inline_md(line.trim_end()), hard_break)
        })
        .collect();
    let n = parts.len();
    let mut inner = String::new();
    for (idx, (text, hard_break)) in parts.into_iter().enumerate() {
        inner.push_str(&text);
        if hard_break {
            inner.push_str("<br/>");
        } else if idx + 1 < n {
            inner.push('\n');
        }
    }
    if !inner.trim().is_empty() {
        blocks.push(format!("<p>{}</p>", inner));
    }
}

fn fence_open(t: &str) -> Option<String> {
    let n = t.chars().take_while(|c| *c == '`').count();
    if n >= 3 {
        Some(t[n..].trim().to_string())
    } else {
        None
    }
}

fn parse_fence(lines: &[&str], start: usize, lang: &str) -> (String, usize) {
    let mut content_lines: Vec<&str> = Vec::new();
    let mut i = start + 1;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with("```") {
            i += 1;
            break;
        }
        content_lines.push(lines[i]);
        i += 1;
    }
    let content = content_lines.join("\n");
    let esc = html_escape(&content);
    if lang.is_empty() {
        (format!("<pre>{}</pre>", esc), i)
    } else {
        (
            format!("<pre data-lang=\"{}\">{}</pre>", attr_escape(lang), esc),
            i,
        )
    }
}

fn heading_parts(t: &str) -> Option<(usize, &str)> {
    let n = t.chars().take_while(|c| *c == '#').count();
    if n == 0 || n > 6 {
        return None;
    }
    let rest = &t[n..];
    if rest.starts_with(' ') {
        Some((n, rest.trim()))
    } else if rest.is_empty() {
        Some((n, ""))
    } else {
        None
    }
}

fn is_hr(t: &str) -> bool {
    (t.chars().all(|c| c == '-') && t.len() >= 3)
        || (t.chars().all(|c| c == '*') && t.len() >= 3)
        || (t.chars().all(|c| c == '_') && t.len() >= 3)
}

fn parse_blockquote(lines: &[&str], start: usize) -> (String, usize) {
    let mut content: Vec<String> = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if let Some(rest) = t.strip_prefix('>') {
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            content.push(rest.to_string());
            i += 1;
        } else {
            break;
        }
    }
    let inner = inline_md(&content.join("\n"));
    (format!("<blockquote><p>{}</p></blockquote>", inner), i)
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    for c in t.chars() {
        if esc {
            cur.push('\\');
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '|' {
            cells.push(cur.trim().to_string());
            cur = String::new();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn is_delimiter_cell(c: &str) -> bool {
    let c = c.trim();
    let c = c.strip_prefix(':').unwrap_or(c);
    let c = c.strip_suffix(':').unwrap_or(c);
    !c.is_empty() && c.chars().all(|x| x == '-')
}

fn is_table_start(lines: &[&str], i: usize) -> bool {
    let cur = lines.get(i).map(|l| l.trim()).unwrap_or("");
    let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
    if !cur.contains('|') || next.is_empty() || !next.contains('|') {
        return false;
    }
    let cells = split_row(next);
    cells.iter().all(|c| is_delimiter_cell(c)) && cells.iter().any(|c| !c.trim().is_empty())
}

fn parse_table(lines: &[&str], start: usize) -> (String, usize) {
    let header = split_row(lines[start]);
    let mut html = String::from("<table><thead><tr>");
    for h in &header {
        html.push_str(&format!("<th>{}</th>", inline_md(h)));
    }
    html.push_str("</tr></thead>");
    let mut body = String::new();
    let mut j = start + 2;
    while j < lines.len() {
        let l = lines[j];
        if l.trim().is_empty() || !l.contains('|') {
            break;
        }
        body.push_str("<tr>");
        for c in split_row(l) {
            body.push_str(&format!("<td>{}</td>", inline_md(&c)));
        }
        body.push_str("</tr>");
        j += 1;
    }
    if !body.is_empty() {
        html.push_str("<tbody>");
        html.push_str(&body);
        html.push_str("</tbody>");
    }
    html.push_str("</table>");
    (html, j)
}

/// Returns (ordered, marker_len) when the line begins with a list marker.
fn list_marker(t: &str) -> Option<(bool, usize)> {
    for m in ["- ", "* ", "+ "] {
        if t.starts_with(m) {
            return Some((false, m.len()));
        }
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 9 {
        let after = &t[digits..];
        for m in [". ", ") "] {
            if after.starts_with(m) {
                return Some((true, digits + m.len()));
            }
        }
        if after == "." || after == ")" {
            return Some((true, digits + 1));
        }
    }
    None
}

struct MList {
    ordered: bool,
    items: Vec<MItem>,
}

struct MItem {
    content: String,
    children: Vec<MList>,
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn parse_list(lines: &[&str], start: usize, indent: usize) -> (MList, usize) {
    let first = list_marker(lines[start].trim_start()).expect("parse_list called on list line");
    let mut list = MList {
        ordered: first.0,
        items: Vec::new(),
    };
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            break;
        }
        let ind = indent_of(line);
        if ind < indent {
            break;
        }
        let t = line.trim_start();
        if ind == indent {
            let Some((_, mlen)) = list_marker(t) else {
                break;
            };
            let mut content = t[mlen..].trim().to_string();
            let mut children: Vec<MList> = Vec::new();
            i += 1;
            loop {
                if i >= lines.len() {
                    break;
                }
                let l2 = lines[i];
                if l2.trim().is_empty() {
                    break;
                }
                let ind2 = indent_of(l2);
                if ind2 > indent {
                    if list_marker(l2.trim_start()).is_some() {
                        let (sub, ni) = parse_list(lines, i, ind2);
                        children.push(sub);
                        i = ni;
                    } else {
                        // lazy continuation of this item
                        content.push('\n');
                        content.push_str(l2.trim_start());
                        i += 1;
                    }
                } else if ind2 == indent && list_marker(l2.trim_start()).is_some() {
                    break; // next sibling item
                } else {
                    break;
                }
            }
            list.items.push(MItem { content, children });
        } else {
            break;
        }
    }
    (list, i)
}

fn render_list(l: &MList) -> String {
    let tag = if l.ordered { "ol" } else { "ul" };
    let mut s = format!("<{}>", tag);
    for item in &l.items {
        s.push_str("<li>");
        s.push_str(&inline_md(&item.content));
        for ch in &item.children {
            s.push_str(&render_list(ch));
        }
        s.push_str("</li>");
    }
    s.push_str(&format!("</{}>", tag));
    s
}

fn parse_list_block(lines: &[&str], i: usize) -> (String, usize) {
    let indent = indent_of(lines[i]);
    let (list, next) = parse_list(lines, i, indent);
    (render_list(&list), next)
}

// ---------------------------------------------------------------------------
// markdown inline -> storage inline
// ---------------------------------------------------------------------------

fn push_escaped(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(c),
    }
}

fn run_len(chars: &[char], from: usize, ch: char) -> usize {
    let mut n = 0;
    let mut j = from;
    while j < chars.len() && chars[j] == ch {
        n += 1;
        j += 1;
    }
    n
}

fn find_close_run(chars: &[char], from: usize, run: usize) -> Option<usize> {
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '`' {
            let l = run_len(chars, j, '`');
            if l == run {
                return Some(j);
            }
            j += l;
        } else {
            j += 1;
        }
    }
    None
}

fn find_str(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    let p: Vec<char> = pat.chars().collect();
    if p.is_empty() {
        return Some(from);
    }
    let mut j = from;
    while j + p.len() <= chars.len() {
        if chars[j..j + p.len()] == p[..] {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn find_char(chars: &[char], from: usize, c: char) -> Option<usize> {
    (from..chars.len()).find(|&j| chars[j] == c)
}

/// Parses "[text](url)" starting at the '[' at index start.
/// Returns (text, url, index after ')').
fn parse_linkish(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let close_bracket = find_char(chars, start + 1, ']')?;
    if chars.get(close_bracket + 1) != Some(&'(') {
        return None;
    }
    let close_paren = find_char(chars, close_bracket + 2, ')')?;
    let text: String = chars[start + 1..close_bracket].iter().collect();
    let url: String = chars[close_bracket + 2..close_paren].iter().collect();
    Some((text, url, close_paren + 1))
}

fn is_intraword_underscore(chars: &[char], i: usize) -> bool {
    i > 0 && chars[i - 1].is_alphanumeric()
}

fn inline_md(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() => {
                push_escaped(&mut out, chars[i + 1]);
                i += 2;
            }
            '`' => {
                let run = run_len(&chars, i, '`');
                if let Some(close) = find_close_run(&chars, i + run, run) {
                    let inner: String = chars[i + run..close].iter().collect();
                    out.push_str("<code>");
                    out.push_str(&html_escape(&inner));
                    out.push_str("</code>");
                    i = close + run;
                } else {
                    for _ in 0..run {
                        out.push('`');
                    }
                    i += run;
                }
            }
            '!' if i + 1 < chars.len() && chars[i + 1] == '[' => {
                if let Some((alt, url, next)) = parse_linkish(&chars, i + 1) {
                    out.push_str(&format!(
                        "<img src=\"{}\" alt=\"{}\"/>",
                        attr_escape(&url),
                        attr_escape(&alt)
                    ));
                    i = next;
                } else {
                    push_escaped(&mut out, '!');
                    i += 1;
                }
            }
            '[' => {
                if let Some((text, url, next)) = parse_linkish(&chars, i) {
                    out.push_str(&format!(
                        "<a href=\"{}\">{}</a>",
                        attr_escape(&url),
                        inline_md(&text)
                    ));
                    i = next;
                } else {
                    push_escaped(&mut out, '[');
                    i += 1;
                }
            }
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                if let Some(close) = find_str(&chars, i + 2, "**") {
                    let inner: String = chars[i + 2..close].iter().collect();
                    out.push_str("<strong>");
                    out.push_str(&inline_md(&inner));
                    out.push_str("</strong>");
                    i = close + 2;
                } else {
                    out.push_str("**");
                    i += 2;
                }
            }
            '*' => {
                if let Some(close) = find_char(&chars, i + 1, '*') {
                    let inner: String = chars[i + 1..close].iter().collect();
                    out.push_str("<em>");
                    out.push_str(&inline_md(&inner));
                    out.push_str("</em>");
                    i = close + 1;
                } else {
                    push_escaped(&mut out, '*');
                    i += 1;
                }
            }
            '_' if !is_intraword_underscore(&chars, i) => {
                if let Some(close) = find_char(&chars, i + 1, '_') {
                    let inner: String = chars[i + 1..close].iter().collect();
                    out.push_str("<em>");
                    out.push_str(&inline_md(&inner));
                    out.push_str("</em>");
                    i = close + 1;
                } else {
                    push_escaped(&mut out, '_');
                    i += 1;
                }
            }
            _ => {
                push_escaped(&mut out, c);
                i += 1;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_to_markdown() {
        assert_eq!(
            storage_to_markdown("<h1>Title</h1><h3>Sub &amp; more</h3>"),
            "# Title\n\n### Sub & more\n"
        );
    }

    #[test]
    fn paragraph_inline_to_markdown() {
        let md = storage_to_markdown(
            "<p>Hello <strong>world</strong> and <em>stars</em> plus <code>a &amp; b</code></p>",
        );
        assert_eq!(md, "Hello **world** and *stars* plus `a & b`\n");
    }

    #[test]
    fn paragraphs_split() {
        assert_eq!(storage_to_markdown("<p>one</p><p>two</p>"), "one\n\ntwo\n");
    }

    #[test]
    fn heading_markdown_to_storage() {
        assert_eq!(
            markdown_to_storage("# Title\n\nBody text"),
            "<h1>Title</h1>\n<p>Body text</p>"
        );
    }

    #[test]
    fn inline_markdown_to_storage() {
        let s = markdown_to_storage("**b** *i* `c` [t](https://x.com) ![a](i.png)");
        assert_eq!(
            s,
            "<p><strong>b</strong> <em>i</em> <code>c</code> <a href=\"https://x.com\">t</a> <img src=\"i.png\" alt=\"a\"/></p>"
        );
    }

    #[test]
    fn lists_round_trip_preserve_text() {
        let md = "- alpha\n- beta\n    - beta-1";
        let storage = markdown_to_storage(md);
        assert_eq!(
            storage,
            "<ul><li>alpha</li><li>beta<ul><li>beta-1</li></ul></li></ul>"
        );
        let back = storage_to_markdown(&storage);
        assert_eq!(back, "- alpha\n- beta\n    - beta-1\n");
    }

    #[test]
    fn ordered_list_both_directions() {
        let storage = "<ol><li>one</li><li>two<ol><li>two-a</li></ol></li></ol>";
        let md = storage_to_markdown(storage);
        assert_eq!(md, "1. one\n2. two\n    1. two-a\n");
        assert_eq!(markdown_to_storage(&md), storage);
    }

    #[test]
    fn code_fence_round_trip() {
        let md = "```rust\nfn main() {}\n```";
        let storage = markdown_to_storage(md);
        assert_eq!(storage, "<pre data-lang=\"rust\">fn main() {}</pre>");
        assert_eq!(
            storage_to_markdown(&storage),
            "```rust\nfn main() {}\n```\n"
        );
    }

    #[test]
    fn blockquote_round_trip() {
        assert_eq!(
            markdown_to_storage("> quoted text"),
            "<blockquote><p>quoted text</p></blockquote>"
        );
        assert_eq!(
            storage_to_markdown("<blockquote><p>quoted text</p></blockquote>"),
            "> quoted text\n"
        );
    }

    #[test]
    fn hr_round_trip() {
        assert_eq!(markdown_to_storage("---"), "<hr/>");
        assert_eq!(storage_to_markdown("<hr/>"), "---\n");
    }

    #[test]
    fn table_round_trip() {
        let storage = "<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td>a</td><td>1</td></tr></tbody></table>";
        let md = storage_to_markdown(storage);
        assert_eq!(md, "| Name | Value |\n| --- | --- |\n| a | 1 |\n");
        assert_eq!(markdown_to_storage(&md), storage);
    }

    #[test]
    fn link_and_image_storage_to_markdown() {
        assert_eq!(
            storage_to_markdown("<p><a href=\"https://ex.com\">Ex</a></p>"),
            "[Ex](https://ex.com)\n"
        );
        assert_eq!(
            storage_to_markdown("<p><img src=\"s.png\" alt=\"pic\"/></p>"),
            "![pic](s.png)\n"
        );
    }

    #[test]
    fn br_to_hard_break() {
        assert_eq!(storage_to_markdown("<p>a<br/>b</p>"), "a  \nb\n");
        assert_eq!(markdown_to_storage("a  \nb"), "<p>a<br/>b</p>");
    }

    #[test]
    fn ac_link_becomes_text() {
        let storage = "<p>See <ac:link><ri:page ri:content-title=\"Home Page\"/><ac:plain-text-link-body><![CDATA[Home]]></ac:plain-text-link-body></ac:link></p>";
        assert_eq!(storage_to_markdown(storage), "See Home\n");
    }

    #[test]
    fn ac_link_without_body_uses_page_title() {
        let storage = "<ac:link><ri:page ri:content-title=\"Home Page\"/></ac:link>";
        assert_eq!(storage_to_markdown(storage), "Home Page\n");
    }

    #[test]
    fn ac_image_attachment() {
        assert_eq!(
            storage_to_markdown("<ac:image><ri:attachment ri:filename=\"shot.png\"/></ac:image>"),
            "![](shot.png)\n"
        );
    }

    #[test]
    fn structured_macro_to_blockquote() {
        let storage = "<ac:structured-macro ac:name=\"info\"><ac:rich-text-body><p>Be careful</p></ac:rich-text-body></ac:structured-macro>";
        assert_eq!(storage_to_markdown(storage), "> [!INFO]\n> Be careful\n");
    }

    #[test]
    fn text_with_markdown_chars_round_trips() {
        let storage = "<p>5 * 3 &lt; 15, a_b &amp; [x]</p>";
        let md = storage_to_markdown(storage);
        assert!(md.contains("\\*"));
        let back = markdown_to_storage(&md);
        assert!(back.contains("5 * 3 &lt; 15"));
        assert!(back.contains("a_b"));
        assert!(back.contains("[x]"));
    }

    #[test]
    fn unknown_tags_keep_text() {
        assert_eq!(
            storage_to_markdown("<p><span>keep</span> me</p>"),
            "keep me\n"
        );
    }

    #[test]
    fn round_trip_preserves_core_text() {
        let md = "# Title\n\nIntro paragraph text.\n\n- item one\n- item two\n\nFinal words.";
        let storage = markdown_to_storage(md);
        let back = storage_to_markdown(&storage);
        assert!(back.contains("# Title"));
        assert!(back.contains("Intro paragraph text."));
        assert!(back.contains("- item one"));
        assert!(back.contains("- item two"));
        assert!(back.contains("Final words."));
    }

    #[test]
    fn pre_storage_with_language() {
        assert_eq!(
            storage_to_markdown("<pre data-lang=\"python\">print(1)</pre>"),
            "```python\nprint(1)\n```\n"
        );
    }
}
