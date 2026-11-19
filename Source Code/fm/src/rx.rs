//! A small backtracking regular expression engine for batch rename (no extra crate, a few KB of code).
//! Syntax: literals, `.`, `[a-z0-9_]`, `[^...]`, `\d \w \s \D \W \S \b \B`, `\.` style escapes, `^ $`,
//! groups `( )`, `(?: )`, alternation `|`, quantifiers `* + ? {n} {n,} {n,m}` and their lazy forms (`*?` ...).
//! Matching is case-insensitive by default (like Windows names); start the pattern with `(?-i)` for exact case.
use crate::util::Res;

#[derive(Clone, Debug)]
enum Item {
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

#[derive(Clone, Debug)]
enum Node {
    Char(char),
    Any,
    Class(Vec<Item>, bool),
    Start,
    End,
    WordB(bool),
    Group(Box<Node>, Option<usize>),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat(Box<Node>, usize, usize, bool),
}

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct Regex {
    root: Node,
    groups: usize,
    icase: bool,
}

type Caps = Vec<Option<(usize, usize)>>;

struct Parser {
    c: Vec<char>,
    i: usize,
    groups: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn alt(&mut self) -> Res<Node> {
        let mut alts = vec![self.concat()?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.concat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { Node::Alt(alts) })
    }
    fn concat(&mut self) -> Res<Node> {
        let mut v = vec![];
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                break;
            }
            let atom = self.atom()?;
            v.push(self.quant(atom)?);
        }
        Ok(Node::Concat(v))
    }
    fn number(&mut self) -> Option<usize> {
        let s = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if s == self.i { None } else { self.c[s..self.i].iter().collect::<String>().parse().ok() }
    }
    fn quant(&mut self, atom: Node) -> Res<Node> {
        let (min, max) = match self.peek() {
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('?') => (0, 1),
            Some('{') => {
                let save = self.i;
                self.i += 1;
                let Some(a) = self.number() else {
                    self.i = save;
                    return Ok(atom); // a literal '{'
                };
                let b = if self.peek() == Some(',') {
                    self.i += 1;
                    self.number().unwrap_or(usize::MAX)
                } else {
                    a
                };
                if self.peek() != Some('}') {
                    self.i = save;
                    return Ok(atom);
                }
                if b < a {
                    return Err("bad {m,n} in pattern".into());
                }
                (a, b)
            }
            _ => return Ok(atom),
        };
        self.i += 1;
        let greedy = if self.peek() == Some('?') {
            self.i += 1;
            false
        } else {
            true
        };
        if matches!(atom, Node::Start | Node::End | Node::WordB(_)) {
            return Err("nothing to repeat in pattern".into());
        }
        Ok(Node::Repeat(Box::new(atom), min, max, greedy))
    }
    fn escape_item(&mut self, inside_class: bool) -> Res<Result<Item, Node>> {
        let Some(e) = self.peek() else { return Err("pattern ends with '\\'".into()) };
        self.i += 1;
        Ok(match e {
            'd' => Ok(Item::Digit(true)),
            'D' => Ok(Item::Digit(false)),
            'w' => Ok(Item::Word(true)),
            'W' => Ok(Item::Word(false)),
            's' => Ok(Item::Space(true)),
            'S' => Ok(Item::Space(false)),
            'b' if !inside_class => Err(Node::WordB(true)),
            'B' if !inside_class => Err(Node::WordB(false)),
            't' => Err(Node::Char('\t')),
            'n' => Err(Node::Char('\n')),
            c if c.is_ascii_alphanumeric() => return Err(format!("unknown escape \\{c} in pattern")),
            c => Err(Node::Char(c)),
        })
    }
    fn atom(&mut self) -> Res<Node> {
        let ch = self.peek().unwrap();
        self.i += 1;
        Ok(match ch {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => {
                let idx = if self.c[self.i..].starts_with(&['?', ':']) {
                    self.i += 2;
                    None
                } else {
                    self.groups += 1;
                    Some(self.groups)
                };
                let inner = self.alt()?;
                if self.peek() != Some(')') {
                    return Err("missing ')' in pattern".into());
                }
                self.i += 1;
                Node::Group(Box::new(inner), idx)
            }
            ')' => return Err("unbalanced ')' in pattern".into()),
            '*' | '+' | '?' => return Err(format!("nothing to repeat before '{ch}' in pattern")),
            '[' => self.class()?,
            '\\' => match self.escape_item(false)? {
                Ok(item) => Node::Class(vec![item], false),
                Err(n) => n,
            },
            c => Node::Char(c),
        })
    }
    fn class(&mut self) -> Res<Node> {
        let neg = self.peek() == Some('^');
        if neg {
            self.i += 1;
        }
        let mut items = vec![];
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return Err("missing ']' in pattern".into()) };
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            self.i += 1;
            let lo = if c == '\\' {
                match self.escape_item(true)? {
                    Ok(item) => {
                        items.push(item);
                        continue;
                    }
                    Err(Node::Char(x)) => x,
                    Err(_) => return Err("bad escape in [] in pattern".into()),
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.c.get(self.i + 1).is_some_and(|&x| x != ']') {
                self.i += 1;
                let mut hi = self.peek().unwrap();
                self.i += 1;
                if hi == '\\' {
                    hi = self.peek().ok_or("bad range in pattern")?;
                    self.i += 1;
                }
                if hi < lo {
                    return Err("bad range in [] in pattern".into());
                }
                items.push(Item::Range(lo, hi));
            } else {
                items.push(Item::Range(lo, lo));
            }
        }
        Ok(Node::Class(items, neg))
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Regex {
    pub fn new(pattern: &str) -> Res<Regex> {
        let (icase, body) = if let Some(r) = pattern.strip_prefix("(?-i)") {
            (false, r)
        } else if let Some(r) = pattern.strip_prefix("(?i)") {
            (true, r)
        } else {
            (true, pattern)
        };
        let mut p = Parser { c: body.chars().collect(), i: 0, groups: 0 };
        let root = p.alt()?;
        if p.i < p.c.len() {
            return Err("unbalanced ')' in pattern".into());
        }
        Ok(Regex { root, groups: p.groups, icase })
    }

    /// Number of capture groups.
    pub fn groups(&self) -> usize {
        self.groups
    }

    fn eq(&self, a: char, b: char) -> bool {
        a == b || (self.icase && a.to_lowercase().eq(b.to_lowercase()))
    }

    fn item_has(&self, it: &Item, c: char) -> bool {
        match *it {
            Item::Range(lo, hi) => {
                (lo <= c && c <= hi) || (self.icase && (c.to_lowercase().any(|x| lo <= x && x <= hi) || c.to_uppercase().any(|x| lo <= x && x <= hi)))
            }
            Item::Digit(p) => c.is_ascii_digit() == p,
            Item::Word(p) => is_word(c) == p,
            Item::Space(p) => c.is_whitespace() == p,
        }
    }

    fn m(&self, n: &Node, s: &[char], i: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        match n {
            Node::Char(c) => i < s.len() && self.eq(s[i], *c) && k(i + 1, caps),
            Node::Any => i < s.len() && k(i + 1, caps),
            Node::Class(items, neg) => i < s.len() && items.iter().any(|it| self.item_has(it, s[i])) != *neg && k(i + 1, caps),
            Node::Start => i == 0 && k(i, caps),
            Node::End => i == s.len() && k(i, caps),
            Node::WordB(want) => {
                let before = i > 0 && is_word(s[i - 1]);
                let after = i < s.len() && is_word(s[i]);
                (before != after) == *want && k(i, caps)
            }
            Node::Group(inner, idx) => match idx {
                None => self.m(inner, s, i, caps, k),
                Some(g) => {
                    let g = *g;
                    self.m(inner, s, i, caps, &mut |j, c: &mut Caps| {
                        let old = c[g];
                        c[g] = Some((i, j));
                        if k(j, c) {
                            true
                        } else {
                            c[g] = old;
                            false
                        }
                    })
                }
            },
            Node::Concat(v) => self.seq(v, 0, s, i, caps, k),
            Node::Alt(v) => v.iter().any(|a| self.m(a, s, i, caps, k)),
            Node::Repeat(inner, min, max, greedy) => self.rep(inner, *min, *max, *greedy, 0, s, i, caps, k),
        }
    }

    fn seq(&self, v: &[Node], idx: usize, s: &[char], i: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        if idx == v.len() {
            return k(i, caps);
        }
        self.m(&v[idx], s, i, caps, &mut |j, c: &mut Caps| self.seq(v, idx + 1, s, j, c, k))
    }

    #[allow(clippy::too_many_arguments)]
    fn rep(&self, n: &Node, min: usize, max: usize, greedy: bool, count: usize, s: &[char], i: usize, caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool) -> bool {
        if !greedy && count >= min && k(i, caps) {
            return true;
        }
        if count < max {
            let more = self.m(n, s, i, caps, &mut |j, c: &mut Caps| {
                if j == i && count >= min {
                    return false; // an empty repeat would loop for ever
                }
                self.rep(n, min, max, greedy, count + 1, s, j, c, k)
            });
            if more {
                return true;
            }
        }
        greedy && count >= min && k(i, caps)
    }

    /// First match at or after char index `start`: group spans in char indices (0 = whole match).
    pub fn find_chars(&self, s: &[char], start: usize) -> Option<Caps> {
        for st in start..=s.len() {
            let mut caps: Caps = vec![None; self.groups + 1];
            if self.m(&self.root, s, st, &mut caps, &mut |j, c: &mut Caps| {
                c[0] = Some((st, j));
                true
            }) {
                return Some(caps);
            }
        }
        None
    }

    /// First match in `s` at or after char index `start`, as strings (None for groups that did not take part).
    pub fn find(&self, s: &str, start: usize) -> Option<Vec<Option<String>>> {
        let ch: Vec<char> = s.chars().collect();
        let caps = self.find_chars(&ch, start)?;
        Some(caps.iter().map(|c| c.map(|(a, b)| ch[a..b].iter().collect())).collect())
    }

    pub fn is_match(&self, s: &str) -> bool {
        self.find(s, 0).is_some()
    }

    /// Replace the first match (or every match with `all`) by what `f` builds from the groups.
    pub fn replace(&self, s: &str, f: impl Fn(&[Option<String>]) -> String, all: bool) -> String {
        let ch: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut pos = 0;
        while pos <= ch.len() {
            let Some(caps) = self.find_chars(&ch, pos) else { break };
            let (a, b) = caps[0].unwrap();
            out.extend(&ch[pos..a]);
            let strs: Vec<Option<String>> = caps.iter().map(|c| c.map(|(x, y)| ch[x..y].iter().collect())).collect();
            out.push_str(&f(&strs));
            if b == a {
                if a < ch.len() {
                    out.push(ch[a]);
                }
                pos = a + 1;
            } else {
                pos = b;
            }
            if !all {
                break;
            }
        }
        if pos < ch.len() {
            out.extend(&ch[pos..]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(p: &str, s: &str) -> Option<String> {
        Regex::new(p).unwrap().find(s, 0).and_then(|c| c[0].clone())
    }
    #[test]
    fn basics() {
        assert_eq!(m("abc", "xxabcxx").as_deref(), Some("abc"));
        assert_eq!(m("^a.c$", "abc").as_deref(), Some("abc"));
        assert_eq!(m("^a.c$", "abcd"), None);
        assert_eq!(m(r"\d+", "img_0042.jpg").as_deref(), Some("0042"));
        assert_eq!(m(r"[a-c]+", "zzBCAzz").as_deref(), Some("BCA")); // case-insensitive by default
        assert_eq!(m(r"(?-i)[a-c]+", "zzBCAzz"), None);
        assert_eq!(m(r"(\w+)\.(jpe?g|png)$", "Photo 1.JPG").as_deref(), Some("1.JPG"));
        assert_eq!(m(r"a{2,3}", "caaaab").as_deref(), Some("aaa"));
        assert_eq!(m(r"a+?", "aaa").as_deref(), Some("a"));
        assert_eq!(m(r"\bcat\b", "a cat!").as_deref(), Some("cat"));
        assert_eq!(m(r"(a|ab)(c|bcd)(d*)", "abcd").as_deref(), Some("abcd"));
        assert_eq!(m(r"[^\d]+", "123abc").as_deref(), Some("abc"));
        assert_eq!(m(r"(a*)*b", "aaab").as_deref(), Some("aaab"));
        assert!(Regex::new("(abc").is_err());
        assert!(Regex::new("*a").is_err());
    }
    #[test]
    fn replace() {
        let r = Regex::new(r"(\d+)").unwrap();
        assert_eq!(r.replace("a1b22", |c| format!("<{}>", c[1].as_deref().unwrap()), true), "a<1>b<22>");
        assert_eq!(r.replace("a1b22", |_| "#".into(), false), "a#b22");
        let e = Regex::new("x*").unwrap();
        assert_eq!(e.replace("ab", |_| "-".into(), true), "-a-b-");
    }
}
