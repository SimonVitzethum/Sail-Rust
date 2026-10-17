#![forbid(unsafe_code)]
//! Wadler-style document algebra with the exact layout rules of OCaml's
//! `PPrint` (`PPrintEngine.ml`, ribbon fraction 1.0), because the Lean
//! backend's output layout is defined by it. Documents are immutable and
//! shared (`Arc`); rendering is iterative so deep documents are safe.

use std::sync::Arc;

/// Flat width requirement; `INF` for anything containing a hard line.
type Req = u64;
const INF: Req = u64::MAX;

fn add(a: Req, b: Req) -> Req {
    if a == INF || b == INF {
        INF
    } else {
        a.saturating_add(b)
    }
}

#[derive(Debug)]
enum Node {
    Empty,
    /// Text with its apparent (display) width.
    Text(Box<str>, usize),
    Blank(usize),
    HardLine,
    IfFlat(Doc, Doc),
    Cat(Req, Doc, Doc),
    Nest(Req, usize, Doc),
    Group(Req, Doc),
    Align(Req, Doc),
}

/// A pretty-printing document.
#[derive(Clone, Debug)]
pub struct Doc(Arc<Node>);

impl Drop for Doc {
    fn drop(&mut self) {
        // Unlink deep chains iteratively (left-nested `Cat` towers).
        let Some(node) = Arc::get_mut(&mut self.0) else {
            return;
        };
        let mut pending: Vec<Doc> = Vec::new();
        take_children(node, &mut pending);
        while let Some(mut doc) = pending.pop() {
            if let Some(node) = Arc::get_mut(&mut doc.0) {
                take_children(node, &mut pending);
            }
        }
    }
}

fn take_children(node: &mut Node, out: &mut Vec<Doc>) {
    let empty = || Doc(Arc::new(Node::Empty));
    match node {
        Node::IfFlat(a, b) | Node::Cat(_, a, b) => {
            out.push(std::mem::replace(a, empty()));
            out.push(std::mem::replace(b, empty()));
        }
        Node::Nest(_, _, a) | Node::Group(_, a) | Node::Align(_, a) => {
            out.push(std::mem::replace(a, empty()));
        }
        _ => {}
    }
}

impl Doc {
    fn new(node: Node) -> Self {
        Doc(Arc::new(node))
    }

    fn req(&self) -> Req {
        match &*self.0 {
            Node::Empty => 0,
            Node::Text(_, width) => *width as Req,
            Node::Blank(n) => *n as Req,
            Node::HardLine => INF,
            Node::IfFlat(flat, _) => flat.req(),
            Node::Cat(req, ..) | Node::Nest(req, ..) | Node::Group(req, _) | Node::Align(req, _) => {
                *req
            }
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(&*self.0, Node::Empty)
    }

    /// `x ^^ y`.
    #[must_use]
    pub fn cat(self, other: Doc) -> Doc {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let req = add(self.req(), other.req());
        Doc::new(Node::Cat(req, self, other))
    }

    /// `x ^/^ y` = `x ^^ break 1 ^^ y`.
    #[must_use]
    pub fn cat_break(self, other: Doc) -> Doc {
        self.cat(brk(1)).cat(other)
    }

    #[must_use]
    pub fn nest(self, indent: usize) -> Doc {
        if self.is_empty() {
            return self;
        }
        let req = self.req();
        Doc::new(Node::Nest(req, indent, self))
    }

    #[must_use]
    pub fn group(self) -> Doc {
        if self.is_empty() {
            return self;
        }
        let req = self.req();
        if req == INF {
            return self;
        }
        Doc::new(Node::Group(req, self))
    }

    #[must_use]
    pub fn align(self) -> Doc {
        if self.is_empty() {
            return self;
        }
        let req = self.req();
        Doc::new(Node::Align(req, self))
    }

    #[must_use]
    pub fn parens(self) -> Doc {
        chr('(').cat(self).cat(chr(')'))
    }

    #[must_use]
    pub fn brackets(self) -> Doc {
        chr('[').cat(self).cat(chr(']'))
    }

    #[must_use]
    pub fn braces(self) -> Doc {
        chr('{').cat(self).cat(chr('}'))
    }

    /// Render with the given width (ribbon fraction 1.0).
    #[must_use]
    pub fn render(&self, width: usize) -> String {
        let mut out = String::new();
        self.render_into(width, &mut out);
        out
    }

    pub fn render_into(&self, width: usize, out: &mut String) {
        let width = width as u64;
        let mut column: u64 = 0;
        let mut last_indent: u64 = 0;
        let mut stack: Vec<(usize, bool, Doc)> = vec![(0, false, self.clone())];
        while let Some((indent, flatten, doc)) = stack.pop() {
            match &*doc.0 {
                Node::Empty => {}
                Node::Text(text, apparent) => {
                    out.push_str(text);
                    column += *apparent as u64;
                }
                Node::Blank(n) => {
                    out.extend(std::iter::repeat(' ').take(*n));
                    column += *n as u64;
                }
                Node::HardLine => {
                    debug_assert!(!flatten, "hard line in flat mode");
                    out.push('\n');
                    out.extend(std::iter::repeat(' ').take(indent));
                    column = indent as u64;
                    last_indent = indent as u64;
                }
                Node::IfFlat(flat, broken) => {
                    stack.push((indent, flatten, if flatten { flat.clone() } else { broken.clone() }));
                }
                Node::Cat(_, a, b) => {
                    stack.push((indent, flatten, b.clone()));
                    stack.push((indent, flatten, a.clone()));
                }
                Node::Nest(_, j, d) => stack.push((indent + *j, flatten, d.clone())),
                Node::Group(req, d) => {
                    let fits = flatten || {
                        let end = add(column, *req);
                        end != INF && end <= width && end <= last_indent + width
                    };
                    stack.push((indent, fits, d.clone()));
                }
                Node::Align(_, d) => stack.push((column as usize, flatten, d.clone())),
            }
        }
    }
}

#[must_use]
pub fn empty() -> Doc {
    Doc::new(Node::Empty)
}

/// `char c` (never a newline).
#[must_use]
pub fn chr(c: char) -> Doc {
    debug_assert!(c != '\n');
    let mut buf = [0u8; 4];
    let text: &str = c.encode_utf8(&mut buf);
    Doc::new(Node::Text(text.into(), 1))
}

/// `string s`: width is the byte length (OCaml `String.length`).
#[must_use]
pub fn string(text: &str) -> Doc {
    if text.is_empty() {
        return empty();
    }
    Doc::new(Node::Text(text.into(), text.len()))
}

/// `utf8string s`: width is the number of code points.
#[must_use]
pub fn utf8string(text: &str) -> Doc {
    if text.is_empty() {
        return empty();
    }
    Doc::new(Node::Text(text.into(), text.chars().count()))
}

#[must_use]
pub fn blank(n: usize) -> Doc {
    if n == 0 {
        empty()
    } else {
        Doc::new(Node::Blank(n))
    }
}

#[must_use]
pub fn space() -> Doc {
    Doc::new(Node::Blank(1))
}

#[must_use]
pub fn hardline() -> Doc {
    Doc::new(Node::HardLine)
}

/// `break n`: `n` blanks when flat, a line break otherwise.
#[must_use]
pub fn brk(n: usize) -> Doc {
    Doc::new(Node::IfFlat(blank(n), hardline()))
}

/// `ifflat a b`.
#[must_use]
pub fn ifflat(flat: Doc, broken: Doc) -> Doc {
    if flat.is_empty() && broken.is_empty() {
        return empty();
    }
    Doc::new(Node::IfFlat(flat, broken))
}

#[must_use]
pub fn concat(docs: impl IntoIterator<Item = Doc>) -> Doc {
    docs.into_iter().fold(empty(), Doc::cat)
}

/// `separate sep docs`.
#[must_use]
pub fn separate(sep: &Doc, docs: impl IntoIterator<Item = Doc>) -> Doc {
    let mut acc: Option<Doc> = None;
    for doc in docs {
        acc = Some(match acc {
            None => doc,
            Some(acc) => acc.cat(sep.clone()).cat(doc),
        });
    }
    acc.unwrap_or_else(empty)
}

/// `flow sep docs`: each element after the first is `group (sep ^^ doc)`.
#[must_use]
pub fn flow(sep: &Doc, docs: impl IntoIterator<Item = Doc>) -> Doc {
    let mut acc: Option<Doc> = None;
    for doc in docs {
        acc = Some(match acc {
            None => doc,
            Some(acc) => acc.cat(sep.clone().cat(doc).group()),
        });
    }
    acc.unwrap_or_else(empty)
}

/// `prefix n b x y` = `group (x ^^ nest n (break b ^^ y))`.
#[must_use]
pub fn prefix(n: usize, b: usize, x: Doc, y: Doc) -> Doc {
    x.cat(brk(b).cat(y).nest(n)).group()
}

/// Identity helper kept for symmetry with PPrint's `group`.
#[must_use]
pub fn group_of(d: Doc) -> Doc {
    d.group()
}

#[must_use]
pub fn comma() -> Doc {
    chr(',')
}

#[must_use]
pub fn comma_sp() -> Doc {
    comma().cat(space())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_breaks_only_when_too_wide() {
        let doc = prefix(2, 1, string("else"), string("(foo bar baz)"));
        assert_eq!(doc.render(100), "else (foo bar baz)");
        assert_eq!(doc.render(10), "else\n  (foo bar baz)");
    }

    #[test]
    fn flow_wraps_at_width() {
        let docs: Vec<Doc> = ["aaa", "bbb", "ccc", "ddd"].iter().map(|s| string(s)).collect();
        let doc = flow(&brk(1), docs);
        assert_eq!(doc.render(100), "aaa bbb ccc ddd");
        assert_eq!(doc.render(8), "aaa bbb\nccc ddd");
    }

    #[test]
    fn nest_applies_to_hardlines() {
        let doc = string("def f :=").cat(hardline().cat(string("x")).nest(2));
        assert_eq!(doc.render(100), "def f :=\n  x");
    }

    #[test]
    fn utf8_width_counts_code_points_but_string_counts_bytes() {
        // `←` is 3 bytes, 1 code point.
        let a = prefix(2, 1, string("←"), string("xxxx"));
        let b = prefix(2, 1, utf8string("←"), string("xxxx"));
        assert_eq!(a.render(6), "←\n  xxxx");
        assert_eq!(b.render(6), "← xxxx");
    }

    #[test]
    fn deep_left_nested_documents_render_and_drop() {
        let mut doc = empty();
        for _ in 0..200_000 {
            doc = doc.cat(string("a"));
        }
        assert_eq!(doc.render(100).len(), 200_000);
    }
}
