//! `LanguageService::rename`: the steps that read a renamed field, and only those, rewritten.

use jlens_core::Expression;
use jlens_schema::{FunctionDecl, LanguageService, UnsureReason};

fn rename(text: &str, table: &[(&str, &str)]) -> (String, Vec<(String, String, UnsureReason)>) {
    rename_with(&LanguageService::new(None), text, table)
}

fn rename_with(
    service: &LanguageService,
    text: &str,
    table: &[(&str, &str)],
) -> (String, Vec<(String, String, UnsureReason)>) {
    let expression = Expression::compile(text).unwrap();
    let renamed = service.rename(&expression, table.iter().copied()).unwrap();
    // Each edit replaces a name as written.
    let mut rebuilt = text.to_owned();
    for edit in renamed.edits.iter().rev() {
        rebuilt.replace_range(edit.span.start..edit.span.end, &edit.text);
    }
    assert_eq!(rebuilt, renamed.text);
    Expression::compile(&renamed.text).expect("the rewritten text parses");
    let unsure = renamed
        .unsure
        .iter()
        .map(|u| {
            (
                text[u.span.start..u.span.end].to_owned(),
                u.path.clone(),
                u.reason,
            )
        })
        .collect();
    (renamed.text, unsure)
}

fn renamed(text: &str, table: &[(&str, &str)]) -> String {
    let (text, unsure) = rename(text, table);
    assert_eq!(unsure, [], "{text}");
    text
}

#[test]
fn steps_that_read_the_field_are_renamed() {
    let table = [("orders[].status", "orders[].state")];
    // A relative path in a predicate, and in a nested path.
    assert_eq!(
        renamed("orders[status = 'open'].total", &table),
        "orders[state = 'open'].total"
    );
    assert_eq!(renamed("orders.status", &table), "orders.state");
    assert_eq!(
        renamed("orders[$contains(status, 'x')]", &table),
        "orders[$contains(state, 'x')]"
    );
    // Another parent's field of the same name is left alone.
    assert_eq!(
        renamed("orders.status & invoice.status & status", &table),
        "orders.state & invoice.status & status"
    );
}

#[test]
fn a_parent_kept_in_a_variable() {
    let table = [("order.total", "order.amount")];
    assert_eq!(
        renamed("($o := order; $o.total * 2)", &table),
        "($o := order; $o.amount * 2)"
    );
    assert_eq!(
        renamed("order@$o.items.($o.total)", &table),
        "order@$o.items.($o.amount)"
    );
    assert_eq!(
        renamed(
            "$map(orders, function($o) { $o.total })",
            &[("orders[].total", "orders[].sum")]
        ),
        "$map(orders, function($o) { $o.sum })"
    );
    assert_eq!(
        renamed("lines.qty.%.sku", &[("lines[].sku", "lines[].code")]),
        "lines.qty.%.code"
    );
}

#[test]
fn a_renamed_parent_renames_its_steps_only() {
    let table = [("customer", "client"), ("customer.name", "client.name")];
    assert_eq!(
        renamed("customer.name & customer.address.city", &table),
        "client.name & client.address.city"
    );
    // The table may leave out the fields under a renamed parent.
    assert_eq!(
        renamed("customer.name", &[("customer", "client")]),
        "client.name"
    );
    assert_eq!(
        renamed(
            "customer.name",
            &[("customer", "client"), ("customer.name", "client.fullName")]
        ),
        "client.fullName"
    );
}

#[test]
fn names_are_written_as_jsonata_needs_them() {
    assert_eq!(
        renamed(
            "customer.`first name`",
            &[("customer.`first name`", "customer.givenName")]
        ),
        "customer.givenName"
    );
    assert_eq!(
        renamed(
            "customer.first",
            &[("customer.first", "customer.`first name`")]
        ),
        "customer.`first name`"
    );
    assert_eq!(renamed("a.b", &[("a.b", "a.and")]), "a.`and`");
    // Byte offsets past characters outside ASCII.
    assert_eq!(
        renamed(
            "'é' & `prénom` & nom",
            &[("prénom", "given"), ("nom", "família")]
        ),
        "'é' & given & família"
    );
    // A name that starts with `«` would open a template (found by the `rename` fuzz target);
    // one that holds it further on is read as a name.
    assert_eq!(renamed("a", &[("a", "«a»")]), "`«a»`");
    assert_eq!(renamed("`«a»`", &[("«a»", "a«b»")]), "a«b»");
}

#[test]
fn templates_objects_and_blocks() {
    let table = [("customer.name", "customer.fullName")];
    assert_eq!(
        renamed("«Dear {customer.name},»", &table),
        "«Dear {customer.fullName},»"
    );
    assert_eq!(
        renamed("customer.{name: name}", &table),
        "customer.{fullName: fullName}"
    );
    assert_eq!(
        renamed("customer.{'name': name}", &table),
        "customer.{'name': fullName}"
    );
    assert_eq!(
        renamed("{'n': customer.name}.n", &table),
        "{'n': customer.fullName}.n"
    );
}

#[test]
fn sorts_groups_and_roots() {
    let table = [("orders[].status", "orders[].state")];
    assert_eq!(
        renamed("orders^(>status).total", &table),
        "orders^(>state).total"
    );
    assert_eq!(
        renamed("orders{status: $sum(total)}", &table),
        "orders{state: $sum(total)}"
    );
    assert_eq!(
        renamed("$.orders[0].status & $$.orders#$i[$i > 0].status", &table),
        "$.orders[0].state & $$.orders#$i[$i > 0].state"
    );
    assert_eq!(
        renamed("orders.(status = 'open' ? total : 0)", &table),
        "orders.(state = 'open' ? total : 0)"
    );
    // A transform takes its input whole.
    let (text, unsure) = rename("$ ~> | orders | {'status': 'x'} |", &table);
    assert_eq!(text, "$ ~> | orders | {'status': 'x'} |");
    assert_eq!(
        unsure,
        [("$".to_owned(), "$".to_owned(), UnsureReason::Whole)]
    );
}

#[test]
fn fields_named_as_data_are_listed() {
    use UnsureReason::*;
    let table = [("customer.vip", "customer.gold")];
    let unsure = |text: &str| rename(text, &table).1;
    assert_eq!(
        unsure("$lookup(customer, 'vip')"),
        [("'vip'".to_owned(), "customer.vip".to_owned(), String)]
    );
    assert_eq!(
        unsure("$keys(customer)"),
        [("customer".to_owned(), "customer".to_owned(), Whole)]
    );
    assert_eq!(
        unsure("customer.*"),
        [("*".to_owned(), "customer".to_owned(), Whole)]
    );
    assert_eq!(unsure("**.vip"), [("**".to_owned(), "$".to_owned(), Whole)]);
    assert_eq!(
        unsure("$sift(customer, function($v, $k) { $k = 'vip' })"),
        [("customer".to_owned(), "customer".to_owned(), Whole)]
    );
    // A function of the expression's own takes its argument whole.
    assert_eq!(
        unsure("($f := function($c) { $c.vip }; $f(customer))"),
        [("customer".to_owned(), "customer".to_owned(), Whole)]
    );
    // The value itself, or a field beside it, is not.
    assert_eq!(unsure("$string(customer.vip) & $string(customer.name)"), []);
    // The result of the expression is not listed.
    assert_eq!(rename("customer", &table), ("customer".to_owned(), vec![]));
    // `$eval` may read anything.
    assert_eq!(
        unsure("$eval('customer.vip')"),
        [("$eval('customer.vip')".to_owned(), "$".to_owned(), Lost)]
    );
}

#[test]
fn a_step_over_two_parents_is_listed() {
    let (text, unsure) = rename("(x ? a : b).name", &[("a.name", "a.title")]);
    assert_eq!(text, "(x ? a : b).name");
    assert_eq!(
        unsure,
        [("name".to_owned(), "a.name".to_owned(), UnsureReason::Mixed)]
    );
    // An object the expression builds keeps its names.
    let (_, unsure) = rename("(x ? a : {'name': 1}).name", &[("a.name", "a.title")]);
    assert_eq!(
        unsure,
        [("name".to_owned(), "a.name".to_owned(), UnsureReason::Mixed)]
    );
    let (_, unsure) = rename("$merge([a, {'name': 1}]).name", &[("a.name", "a.title")]);
    assert_eq!(
        unsure,
        [
            (
                "[a, {'name': 1}]".to_owned(),
                "a".to_owned(),
                UnsureReason::Whole
            ),
            ("name".to_owned(), "a.name".to_owned(), UnsureReason::Mixed)
        ]
    );
    // Renamed alike under both, the step is renamed.
    assert_eq!(
        renamed(
            "(x ? a : b).name",
            &[("a.name", "a.title"), ("b.name", "b.title")]
        ),
        "(x ? a : b).title"
    );
}

#[test]
fn declared_functions_read_their_arguments() {
    let service = LanguageService::new(None)
        .with_function(FunctionDecl::new("describe", "Describes a value"));
    let table = [("customer.vip", "customer.gold")];
    let (text, unsure) = rename_with(&service, "$describe(customer.vip) & customer.vip", &table);
    assert_eq!(text, "$describe(customer.gold) & customer.gold");
    assert_eq!(unsure, []);
    // A function not declared reads its context: the whole input.
    let (_, unsure) = rename("$other(customer.vip)", &table);
    assert_eq!(
        unsure,
        [("$other".to_owned(), "$".to_owned(), UnsureReason::Whole)]
    );
}

#[test]
fn variables_are_not_renamed() {
    assert_eq!(
        renamed("$customer.vip & vip", &[("vip", "gold")]),
        "$customer.vip & gold"
    );
}

#[test]
fn an_empty_table_gives_the_text_back() {
    let expression = Expression::compile("customer.name").unwrap();
    let service = LanguageService::new(None);
    let none: [(&str, &str); 0] = [];
    let renamed = service.rename(&expression, none).unwrap();
    assert_eq!(renamed.text, "customer.name");
    assert!(renamed.edits.is_empty() && renamed.unsure.is_empty());
    // Entries that keep every name change nothing either.
    let renamed = service
        .rename(
            &expression,
            [
                ("$", "$"),
                ("customer", "customer"),
                ("customer.name", "customer.name"),
            ],
        )
        .unwrap();
    assert_eq!(renamed.text, "customer.name");
}

#[test]
fn tables_that_are_not_renames_fail() {
    let service = LanguageService::new(None);
    let expression = Expression::compile("a.b").unwrap();
    for table in [
        vec![("a.b", "c.b")],
        vec![("a.b", "a.b.c")],
        vec![("a", "$")],
        vec![("a.b", "a.c"), ("a.b", "a.d")],
        vec![("a.b", "a.c"), ("a.d", "a.c")],
        vec![("a.b", "a.`c`d`")],
        vec![("a", "x"), ("a.b", "a.c")],
    ] {
        assert!(
            service.rename(&expression, table.clone()).is_err(),
            "{table:?}"
        );
    }
}

#[test]
fn json_form() {
    let service = LanguageService::new(None);
    let expression = Expression::compile("$keys(a) & a.b").unwrap();
    let renamed = service.rename(&expression, [("a.b", "a.c")]).unwrap();
    assert_eq!(
        serde_json::to_value(&renamed).unwrap(),
        serde_json::json!({
            "text": "$keys(a) & a.c",
            "edits": [{"span": {"start": 13, "end": 14}, "text": "c"}],
            "unsure": [{"span": {"start": 6, "end": 7}, "path": "a", "reason": "whole"}]
        })
    );
}

/// The example of the schema guide: a rule migrated to a new version of its schema.
#[test]
fn the_example_of_the_guides() -> Result<(), String> {
    use jlens_schema::Schema;
    use std::collections::{BTreeMap, HashMap};

    let version = |lines: &str, qty: &str| {
        Schema::from_json_schema(&serde_json::json!({
            "type": "object",
            "properties": {
                lines: {"type": "array", "x-attribute-id": "L", "items": {
                    "type": "object", "properties": {
                        qty: {"type": "integer", "x-attribute-id": "Q"},
                        "price": {"type": "number", "x-attribute-id": "P"}}}},
                "status": {"type": "string", "x-attribute-id": "S"}
            }
        }))
    };
    let (old, new) = (version("lines", "qty")?, version("items", "quantity")?);
    let before = LanguageService::new(Some(old));
    let after = LanguageService::new(Some(new));
    let rule = Expression::compile("status = 'open' and $sum(lines.(qty * price)) > 100")
        .map_err(|e| e.to_string())?;
    // Was anything the rule uses changed? `lines` and `qty` are gone.
    let changes = before.uses_of(&rule).changes(&after.uses_of(&rule));
    assert!(changes.input.removed.contains_key("lines[].qty"));
    // The table: each old path to the new one with the same key.
    let key = |field: &jlens_schema::ListedField| field.extensions.get("x-attribute-id").cloned();
    let new_paths: HashMap<_, _> = after
        .fields()
        .into_iter()
        .filter_map(|field| Some((key(&field)?.to_string(), field.path)))
        .collect();
    let table: BTreeMap<String, String> = before
        .fields()
        .into_iter()
        .filter_map(|field| {
            Some((
                field.path.clone(),
                new_paths.get(&key(&field)?.to_string())?.clone(),
            ))
        })
        .collect();
    let renamed = before.rename(&rule, &table)?;
    assert_eq!(
        renamed.text,
        "status = 'open' and $sum(items.(quantity * price)) > 100"
    );
    assert!(renamed.unsure.is_empty());
    // The rewritten rule uses nothing the new version does not have.
    let migrated = Expression::compile(&renamed.text).map_err(|e| e.to_string())?;
    let uses = after.uses_of(&migrated);
    assert!(uses.input.iter().flatten().all(|(_, used)| !used.unknown));
    Ok(())
}

#[test]
fn backquotes_are_kept_where_the_name_would_run_on() {
    let table = [("a", "x")];
    // Without its backquotes, the name would run into the operator after it.
    assert_eq!(renamed("`a`and b", &table), "`x`and b");
    assert_eq!(renamed("`a`or(b)", &table), "`x`or(b)");
    assert_eq!(renamed("`a`in[1, 2]", &table), "`x`in[1, 2]");
    // Where an operator character or a space follows, they are not needed.
    assert_eq!(renamed("`a` and b", &table), "x and b");
    assert_eq!(renamed("`a`.b", &table), "x.b");
}

#[test]
fn a_step_written_as_a_string_is_renamed() {
    let table = [("customer.name", "customer.fullName")];
    assert_eq!(renamed("customer.\"name\"", &table), "customer.fullName");
    assert_eq!(
        renamed("customer.'name' & 'name'", &table),
        "customer.fullName & 'name'"
    );
}

#[test]
fn a_step_over_an_object_the_expression_builds_is_listed() {
    let table = [("a.name", "a.title")];
    let mixed = |text: &str| {
        let (out, unsure) = rename(text, &table);
        assert_eq!(out, text);
        assert!(
            unsure.iter().any(|(span, path, reason)| span == "name"
                && path == "a.name"
                && *reason == UnsureReason::Mixed),
            "{text}: {unsure:?}"
        );
    };
    mixed("(x ? a : items{kind: $count($)}).name");
    mixed("(x ? a : items.{kind: $count($)}).name");
    mixed("(x ? a : $clone(b)).name");
    mixed("(x ? a : $sift(b, function($v) { $v })).name");
    mixed("(x ? a : $spread(b)).name");
    mixed("(x ? a : $match('x', /x/)).name");
    mixed("(x ? a : ($ ~> |b|{'c': 1}|)).name");
    mixed("($f := function() { {'name': 1} }; (x ? a : $f())).name");
    mixed("(x ? a : b ~> $f).name");
    mixed("(x ? a : $lookup(b, k)).name");
    // A function that gives a string or a number builds no object.
    assert_eq!(
        renamed("(x ? a : $string(b)).name", &table),
        "(x ? a : $string(b)).title"
    );
}

#[test]
fn paths_with_stray_backquotes_or_dots_fail() {
    let service = LanguageService::new(None);
    let expression = Expression::compile("a.b").unwrap();
    for table in [
        vec![("a", "b`")],
        vec![("a", "```")],
        vec![("`a", "b")],
        vec![("a`b`", "c")],
        vec![("...a", "b")],
        vec![("a..b", "a.c")],
        vec![("a.", "b")],
    ] {
        assert!(
            service.rename(&expression, table.clone()).is_err(),
            "{table:?}"
        );
    }
    // `$.a`, `.a` and `$[].a` are paths.
    assert_eq!(renamed("a", &[("$.a", "b")]), "b");
    assert_eq!(renamed("a", &[(".a", "b")]), "b");
    assert_eq!(renamed("a", &[("$[].a", "$[].b")]), "b");
}
