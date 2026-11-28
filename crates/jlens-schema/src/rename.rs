//! Fields renamed in an expression.
//!
//! [`LanguageService::rename`] rewrites the steps of an expression that read a field a new
//! version of its schema renames, found as [`LanguageService::uses`] finds the fields an
//! expression reads ([`jlens_core::reads::Sites`]), and lists the places it cannot rewrite
//! safely.

use std::collections::{BTreeMap, HashMap};

use jlens_core::Expression;
use jlens_core::ast::Span;
use jlens_core::reads::ReadPath;
use serde::Serialize;

use crate::service::{LanguageService, field_insert, is_word_char};
use crate::shape::parse_path;

/// An expression with renamed fields, given by [`LanguageService::rename`].
///
/// Its JSON form is `{"text": ..., "edits": [{"span": {"start", "end"}, "text"}], "unsure":
/// [{"span", "path", "reason"}]}`, spans in bytes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Renamed {
    /// The rewritten text: the expression's own, with the edits made.
    pub text: String,
    /// Each name replaced, in the order of the text, its span that of the expression's text.
    pub edits: Vec<RenameEdit>,
    /// The places the rename cannot be sure of, in the order of the text: a field renamed there
    /// may be read where no step names it, or named as data. The text is left as it is there.
    pub unsure: Vec<Unsure>,
}

/// A name replaced (see [`Renamed`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenameEdit {
    /// The bytes of the name in the expression: the name of a step, with its backquotes.
    pub span: Span,
    /// What replaces it: the new name, between backquotes when it is not a plain one.
    pub text: String,
}

/// A place the rename cannot be sure of (see [`Renamed`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unsure {
    /// The bytes of the operand in the expression; the whole text for [`UnsureReason::Lost`].
    pub span: Span,
    /// The path concerned, written as [`Schema::shape_at`](crate::Schema::shape_at) reads it:
    /// the value used whole, or the renamed field; `$` for [`UnsureReason::Lost`].
    pub path: String,
    /// Why the rename cannot be sure of it.
    pub reason: UnsureReason,
}

/// Why a rename cannot be sure of a place (see [`Unsure`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnsureReason {
    /// A value that holds a renamed field is used whole: given to a function, which may read
    /// the field or its name (`$keys`, `$sift`, `$each`, `$string`, a function of the
    /// expression or the application), iterated by `*` or `**`, an operand...
    Whole,
    /// A renamed field is named by a string, given to `$lookup`.
    String,
    /// The step reads a renamed field and another one, which keeps its name, or two fields
    /// renamed apart: no single name fits both. The other may be a field of an object the
    /// expression builds or a function gives (a group-by, `$sift`, a function of the
    /// expression's).
    Mixed,
    /// The analysis gave up on part of the expression (`$eval`, a parent it cannot find, a
    /// value with too many places), where a renamed field may be read.
    Lost,
}

/// A name as it replaces the one at `span`: between backquotes when it is not a plain name, or
/// when the text beside it would run into it (`` `a`and b `` read as `xand b`).
fn written_at(source: &str, span: Span, name: &str) -> String {
    let text = field_insert(name);
    let runs_on = |c: Option<char>| c.is_some_and(is_word_char);
    if !text.starts_with('`')
        && (runs_on(source[..span.start].chars().next_back())
            || runs_on(source[span.end..].chars().next()))
    {
        format!("`{name}`")
    } else {
        text
    }
}

/// The table of renames, checked.
struct Table {
    /// The new name of each renamed field, by its old path (field names only).
    renamed: HashMap<Vec<String>, String>,
    /// Each renamed path and how many `[]` follow each of its names, as the table wrote it.
    written: Vec<(Vec<String>, Vec<usize>)>,
}

impl Table {
    fn new<I, A, B>(renames: I) -> Result<Table, String>
    where
        I: IntoIterator<Item = (A, B)>,
        A: AsRef<str>,
        B: AsRef<str>,
    {
        let parse = |path: &str| {
            let segments = parse_path(path).ok_or_else(|| format!("`{path}` is not a path"))?;
            let mut names = Vec::new();
            let mut arrays: Vec<usize> = Vec::new();
            for (name, count) in segments {
                match name {
                    Some(name) => {
                        names.push(name);
                        arrays.push(count);
                    }
                    // Items of the root: a step over an array reads its items' fields.
                    None if names.is_empty() => {}
                    None => return Err(format!("`{path}` is not a path")),
                }
            }
            Ok::<_, String>((names, arrays))
        };
        let mut entries: BTreeMap<Vec<String>, (Vec<String>, Vec<usize>, String)> = BTreeMap::new();
        for (old, new) in renames {
            let (old, new) = (old.as_ref(), new.as_ref());
            let (old_names, arrays) = parse(old)?;
            let (new_names, _) = parse(new)?;
            if old_names.is_empty() != new_names.is_empty() {
                return Err(format!(
                    "`{old}` cannot be renamed `{new}`: one is the root"
                ));
            }
            if old_names.len() != new_names.len() {
                return Err(format!(
                    "`{old}` cannot be renamed `{new}`: a rename keeps the field under its parent"
                ));
            }
            if let Some(name) = new_names.last().filter(|name| name.contains('`')) {
                return Err(format!(
                    "`{name}` cannot be written as a name: it holds a backquote"
                ));
            }
            match entries.get(&old_names) {
                Some((other, _, _)) if *other != new_names => {
                    return Err(format!("`{old}` is renamed twice"));
                }
                Some(_) => {}
                None => {
                    entries.insert(old_names, (new_names, arrays, old.to_owned()));
                }
            }
        }
        let renamed: HashMap<Vec<String>, String> = entries
            .iter()
            .filter(|(old, (new, _, _))| old.last() != new.last())
            .filter_map(|(old, (new, _, _))| Some((old.clone(), new.last()?.clone())))
            .collect();
        let table = Table {
            written: entries
                .iter()
                .filter(|(old, _)| renamed.contains_key(*old))
                .map(|(old, (_, arrays, _))| (old.clone(), arrays.clone()))
                .collect(),
            renamed,
        };
        // Each entry must be its field renamed, under its parent as the table renames it.
        let mut targets: HashMap<Vec<String>, &str> = HashMap::new();
        for (old, (new, _, written)) in &entries {
            if table.map(old) != *new {
                return Err(format!(
                    "`{written}` cannot be renamed so: a rename keeps the field under its \
                     parent, renamed as the table renames it"
                ));
            }
            if let Some(other) = targets.insert(new.clone(), written) {
                return Err(format!(
                    "`{other}` and `{written}` are renamed to the same path"
                ));
            }
        }
        Ok(table)
    }

    /// The path of a field once each name on its way is renamed.
    fn map(&self, path: &[String]) -> Vec<String> {
        let mut out = Vec::with_capacity(path.len());
        for (i, name) in path.iter().enumerate() {
            out.push(self.renamed.get(&path[..=i]).unwrap_or(name).clone());
        }
        out
    }

    /// The path of a value on the way to a renamed field, as the table writes it.
    fn write(&self, path: &[String]) -> String {
        if path.is_empty() {
            return "$".to_owned();
        }
        let arrays = self
            .written
            .iter()
            .find(|(old, _)| old.starts_with(path))
            .map(|(_, arrays)| arrays.as_slice())
            .unwrap_or_default();
        let mut out = String::new();
        for (i, name) in path.iter().enumerate() {
            if i > 0 {
                out.push('.');
            }
            out.push_str(&field_insert(name));
            // The items of the value itself are not on its way.
            if i + 1 < path.len() {
                out.push_str(&"[]".repeat(arrays.get(i).copied().unwrap_or(0)));
            }
        }
        out
    }

    /// Whether a renamed field is under the value at `path`.
    fn holds_renamed(&self, path: &[String]) -> bool {
        self.renamed
            .keys()
            .any(|old| old.len() > path.len() && old.starts_with(path))
    }
}

impl LanguageService {
    /// Renames fields of the input in an expression: given a table of paths, each old path to
    /// its new one, as [`Schema::shape_at`](crate::Schema::shape_at) reads them (`$` for the
    /// input, `lines[].sku`), it replaces each step that reads a renamed field, found as
    /// [`uses_of`](Self::uses_of) finds the fields an expression reads, and keeps the rest of
    /// the text as it is written.
    ///
    /// A step is renamed where it reads the field, whatever the path that leads there: a
    /// relative path in a predicate, a parent kept in a variable. The same name under another
    /// parent, or of a value the expression builds, is left alone. An entry renames the last
    /// name of its path; a field whose parent is renamed is renamed under its parent without
    /// an entry of its own, and an entry for it must give it under the new parent. A table
    /// built from the two versions' [`fields`](Self::fields) has an entry for every field,
    /// most of them kept.
    ///
    /// Where the rename cannot be sure of the text, it leaves it as it is and lists the place
    /// ([`Renamed::unsure`]): a value that holds a renamed field used whole, as `$keys`, `*` or
    /// a function of the application take it, a renamed field named by a string given to
    /// `$lookup`, a step that reads both a renamed field and another.
    ///
    /// The table is of the input's fields; variables' are not renamed. It fails, saying why,
    /// on a path it cannot read, an entry that moves a field to another parent, a field renamed
    /// twice or two renamed to the same path, and a name holding a backquote, which JSONata
    /// cannot write.
    ///
    /// ```
    /// use jlens_core::Expression;
    /// use jlens_schema::LanguageService;
    ///
    /// let service = LanguageService::new(None);
    /// let expression = Expression::compile(
    ///     "$sum(lines[status = 'open'].(qty * price)) + $count($keys(customer))",
    /// )
    /// .unwrap();
    /// let renamed = service
    ///     .rename(
    ///         &expression,
    ///         [("lines", "items"), ("lines[].qty", "items[].quantity"), ("customer.vip", "customer.gold")],
    ///     )
    ///     .unwrap();
    /// assert_eq!(
    ///     renamed.text,
    ///     "$sum(items[status = 'open'].(quantity * price)) + $count($keys(customer))"
    /// );
    /// // `$keys` takes the names of `customer`'s fields, `vip` among them.
    /// assert_eq!(renamed.unsure[0].path, "customer");
    /// ```
    pub fn rename<I, A, B>(&self, expression: &Expression, renames: I) -> Result<Renamed, String>
    where
        I: IntoIterator<Item = (A, B)>,
        A: AsRef<str>,
        B: AsRef<str>,
    {
        let table = Table::new(renames)?;
        let source = expression.source();
        let mut edits = Vec::new();
        let mut unsure = Vec::new();
        if table.renamed.is_empty() {
            return Ok(Renamed {
                text: source.to_owned(),
                edits,
                unsure,
            });
        }
        let sites = expression.sites_with(&self.host_bindings());
        let input = |read: &ReadPath| read.binding.is_none();
        for site in &sites.fields {
            // The new name of each field the step reads, `None` for one kept: a binding's, an
            // object's the expression builds.
            let mut renamed: Vec<Option<&String>> = site
                .reads
                .iter()
                .map(|read| {
                    input(read)
                        .then(|| table.renamed.get(&read.fields))
                        .flatten()
                })
                .collect();
            if site.built {
                renamed.push(None);
            }
            let Some(first) = renamed.iter().flatten().next() else {
                continue;
            };
            let path = site
                .reads
                .iter()
                .find(|read| input(read) && table.renamed.contains_key(&read.fields))
                .map(|read| table.write(&read.fields))
                .unwrap_or_default();
            let reason = if site.string {
                Some(UnsureReason::String)
            } else if renamed.iter().any(|new| *new != Some(*first)) {
                Some(UnsureReason::Mixed)
            } else {
                None
            };
            match reason {
                Some(reason) => unsure.push(Unsure {
                    span: site.span,
                    path,
                    reason,
                }),
                None => edits.push(RenameEdit {
                    span: site.span,
                    text: written_at(source, site.span, first),
                }),
            }
        }
        for site in &sites.wholes {
            if input(&site.path) && table.holds_renamed(&site.path.fields) {
                let item = Unsure {
                    span: site.span,
                    path: table.write(&site.path.fields),
                    reason: UnsureReason::Whole,
                };
                if !unsure.contains(&item) {
                    unsure.push(item);
                }
            }
        }
        if !sites.complete {
            unsure.push(Unsure {
                span: Span::new(0, source.len()),
                path: "$".to_owned(),
                reason: UnsureReason::Lost,
            });
        }
        edits.sort_by_key(|edit| edit.span.start);
        unsure.sort_by_key(|item| (item.span.start, item.span.end));
        let mut text = String::with_capacity(source.len());
        let mut at = 0;
        for edit in &edits {
            text.push_str(&source[at..edit.span.start]);
            text.push_str(&edit.text);
            at = edit.span.end;
        }
        text.push_str(&source[at..]);
        Ok(Renamed {
            text,
            edits,
            unsure,
        })
    }
}
