//! Runs the JavaScript of PDF forms: calculations, number/date formats, checks of what was typed,
//! and buttons — the part of Acrobat's JavaScript that forms rely on.
//!
//! **Sandbox.** The scripts run in [Boa](https://boajs.dev), a JavaScript engine written in Rust,
//! with *no host functions at all*: the Acrobat objects (`this.getField`, `event`, `app`, `util`,
//! the `AF…` functions) are plain JavaScript in `prelude.js`, working on a copy of the field values
//! passed in as JSON. A script can therefore only change field values and their hidden/read-only
//! flags, and show messages. It cannot read or write files, reach the network, open links, other
//! documents or windows; such calls (`submitForm`, `app.launchURL`, `mailDoc`, `print`, …) do nothing
//! and are reported in [`Outcome::blocked`]. Loops and recursion are limited, and every run starts
//! from a fresh engine, so nothing a script does survives to the next run.
//!
//! The caller decides *whether* to run anything at all (BergPDF asks the person per document) and
//! writes the [`Outcome`] back into the document as one undoable step.

use boa_engine::{Context, Source};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const PRELUDE: &str = include_str!("prelude.js");

/// Kind of field, as Acrobat's `field.type` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "checkbox")]
    Checkbox,
    #[serde(rename = "radiobutton")]
    Radio,
    #[serde(rename = "combobox")]
    Combo,
    #[serde(rename = "listbox")]
    List,
    #[serde(rename = "button")]
    Button,
    #[serde(rename = "signature")]
    Signature,
}

/// The scripts of a field; empty = none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scripts {
    pub calculate: String,
    pub validate: String,
    pub format: String,
    pub keystroke: String,
    pub action: String,
}

/// One field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Field {
    /// Fully qualified name.
    pub name: String,
    /// Kind.
    pub kind: Kind,
    /// Value as stored (`Off` or the on-state for boxes).
    pub value: String,
    /// What `resetForm` sets.
    pub default_value: String,
    pub read_only: bool,
    pub required: bool,
    pub hidden: bool,
    /// Options of a list or drop-down: `[export value, shown text]`.
    pub options: Vec<(String, String)>,
    /// On-states of the widgets of a check box or radio group.
    pub export_values: Vec<String>,
    /// Scripts.
    pub scripts: Scripts,
}

/// The form.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Form {
    pub fields: Vec<Field>,
    /// Names of the fields with a calculation, in calculation order.
    pub order: Vec<String>,
    /// Document-level scripts, run first (they usually define helper functions).
    pub document: Vec<String>,
    /// `this.numPages`.
    pub pages: usize,
    /// `this.documentFileName`.
    pub file_name: String,
}

impl Form {
    /// Whether the form has any script to run.
    pub fn has_scripts(&self) -> bool {
        !self.document.is_empty()
            || self.fields.iter().any(|f| {
                let s = &f.scripts;
                !(s.calculate.is_empty()
                    && s.validate.is_empty()
                    && s.format.is_empty()
                    && s.keystroke.is_empty()
                    && s.action.is_empty())
            })
    }
}

/// What happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A field got a new value (typed, chosen or toggled).
    Commit { field: String, value: String },
    /// A button was clicked.
    Click { field: String },
    /// Calculate and format everything (when scripts are switched on, or "Calculate Now").
    CalculateAll,
}

#[derive(Serialize)]
struct EventJson<'a> {
    kind: &'a str,
    field: &'a str,
    value: &'a str,
}

#[derive(Serialize)]
struct Input<'a> {
    #[serde(flatten)]
    form: &'a Form,
    event: EventJson<'a>,
}

/// A field's new state; `None` = unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Change {
    /// New value.
    pub value: Option<String>,
    /// Text a format script wants shown instead of the value.
    pub display: Option<String>,
    pub hidden: Option<bool>,
    pub read_only: Option<bool>,
    pub required: Option<bool>,
}

/// The result of a run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Outcome {
    /// Changed fields by name.
    pub changes: BTreeMap<String, Change>,
    /// Messages from `app.alert`.
    pub alerts: Vec<String>,
    /// Script errors (the script stopped; the rest went on).
    pub errors: Vec<String>,
    /// The new value was refused by the field's keystroke or validate script (nothing changed).
    pub rejected: Option<String>,
    /// Things a script asked for that BergPDF never does (`submitForm`, `launchURL`, …).
    pub blocked: Vec<String>,
}

/// Limits of one run.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Iterations of any single loop.
    pub loop_iterations: u64,
    /// Nested function calls.
    pub recursion: usize,
    /// Total size of all scripts, in bytes.
    pub source_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            loop_iterations: 2_000_000,
            recursion: 400,
            source_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Why a run could not happen at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The scripts are larger than [`Limits::source_bytes`].
    TooLarge,
    /// A script ran too long (an endless loop) or too deep; nothing was changed.
    Limit,
    /// The engine stopped (the prelude failed); the message says why.
    Engine(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::TooLarge => write!(f, "the form's scripts are too large to run"),
            RunError::Limit => write!(
                f,
                "a script of the form ran too long or too deep and was stopped; nothing was changed"
            ),
            RunError::Engine(m) => write!(f, "the form's scripts stopped: {m}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Run the form's scripts for `event` and return what they changed.
pub fn run(form: &Form, event: &Event, limits: Limits) -> Result<Outcome, RunError> {
    let size: usize = form.document.iter().map(String::len).sum::<usize>()
        + form
            .fields
            .iter()
            .map(|f| {
                let s = &f.scripts;
                s.calculate.len()
                    + s.validate.len()
                    + s.format.len()
                    + s.keystroke.len()
                    + s.action.len()
            })
            .sum::<usize>();
    if size > limits.source_bytes {
        return Err(RunError::TooLarge);
    }
    let ev = match event {
        Event::Commit { field, value } => EventJson {
            kind: "commit",
            field,
            value,
        },
        Event::Click { field } => EventJson {
            kind: "click",
            field,
            value: "",
        },
        Event::CalculateAll => EventJson {
            kind: "all",
            field: "",
            value: "",
        },
    };
    let input = serde_json::to_string(&Input { form, event: ev })
        .map_err(|e| RunError::Engine(e.to_string()))?;
    // A JSON string is also a valid JavaScript string literal.
    let literal = serde_json::to_string(&input).map_err(|e| RunError::Engine(e.to_string()))?;
    let program = format!("var __INPUT = {literal};\n{PRELUDE}");
    let mut ctx = Context::default();
    {
        let rl = ctx.runtime_limits_mut();
        rl.set_loop_iteration_limit(limits.loop_iterations);
        rl.set_recursion_limit(limits.recursion);
    }
    // Limits are not catchable inside JavaScript: the whole run stops and changes nothing.
    let value = ctx
        .eval(Source::from_bytes(program.as_bytes()))
        .map_err(|e| {
            let m = e.to_string();
            if m.contains("RuntimeLimitError") || m.contains("Maximum call stack") {
                RunError::Limit
            } else {
                RunError::Engine(m)
            }
        })?;
    let text = value
        .to_string(&mut ctx)
        .map_err(|e| RunError::Engine(e.to_string()))?
        .to_std_string_escaped();
    serde_json::from_str(&text).map_err(|e| RunError::Engine(format!("unreadable result: {e}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn field(name: &str, kind: Kind, value: &str) -> Field {
        Field {
            name: name.into(),
            kind,
            value: value.into(),
            default_value: String::new(),
            read_only: false,
            required: false,
            hidden: false,
            options: Vec::new(),
            export_values: Vec::new(),
            scripts: Scripts::default(),
        }
    }

    fn invoice() -> Form {
        let mut total = field("total", Kind::Text, "");
        total.scripts.calculate =
            "AFSimple_Calculate(\"SUM\", new Array(\"a\", \"b\", \"c\"));".into();
        total.scripts.format = "AFNumber_Format(2, 2, 0, 0, \"€ \", true);".into();
        let mut vat = field("vat", Kind::Text, "");
        vat.scripts.calculate = "event.value = this.getField(\"total\").value * 0.21;".into();
        vat.scripts.format = "AFNumber_Format(2, 0, 0, 0, \"\", true);".into();
        let mut a = field("a", Kind::Text, "10");
        a.scripts.keystroke = "AFNumber_Keystroke(2, 0, 0, 0, \"\", true);".into();
        a.scripts.validate = "AFRange_Validate(true, 0, true, 1000);".into();
        Form {
            fields: vec![
                a,
                field("b", Kind::Text, "2,5"),
                field("c", Kind::Text, ""),
                total,
                vat,
            ],
            order: vec!["total".into(), "vat".into()],
            document: Vec::new(),
            pages: 1,
            file_name: "invoice.pdf".into(),
        }
    }

    #[test]
    fn calculations_and_formats_follow_a_change() {
        let out = run(
            &invoice(),
            &Event::Commit {
                field: "a".into(),
                value: "997.5".into(),
            },
            Limits::default(),
        )
        .unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(out.changes.contains_key("a"), "{out:?}");
        assert_eq!(out.changes["a"].value.as_deref(), Some("997.5"));
        // 997.5 + "2,5" (a decimal comma) + empty = 1000.
        assert_eq!(out.changes["total"].value.as_deref(), Some("1000"));
        assert_eq!(out.changes["total"].display.as_deref(), Some("€ 1.000,00"));
        assert_eq!(out.changes["vat"].value.as_deref(), Some("210"));
        assert_eq!(out.changes["vat"].display.as_deref(), Some("210.00"));
    }

    #[test]
    fn keystroke_and_validate_scripts_refuse_bad_values() {
        let out = run(
            &invoice(),
            &Event::Commit {
                field: "a".into(),
                value: "twelve".into(),
            },
            Limits::default(),
        )
        .unwrap();
        assert_eq!(out.rejected.as_deref(), Some("a"));
        assert!(out.changes.is_empty());
        assert!(out.alerts[0].contains("does not match the format"));
        let out = run(
            &invoice(),
            &Event::Commit {
                field: "a".into(),
                value: "5000".into(),
            },
            Limits::default(),
        )
        .unwrap();
        assert_eq!(out.rejected.as_deref(), Some("a"));
        assert!(out.alerts[0].contains("less than or equal to 1000"));
    }

    #[test]
    fn buttons_document_scripts_and_flags() {
        let mut form = invoice();
        form.document = vec!["function twice(x) { return 2 * x; }".into()];
        let mut go = field("go", Kind::Button, "");
        go.scripts.action = r#"
            var c = this.getField("c");
            c.value = twice(21);
            this.getField("b").display = display.hidden;
            this.getField("a").readonly = true;
            app.alert({cMsg: "Done", nIcon: 3});
            this.submitForm("https://example.com");
            app.launchURL("https://example.com", true);
        "#
        .into();
        form.fields.push(go);
        let out = run(
            &form,
            &Event::Click { field: "go".into() },
            Limits::default(),
        )
        .unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.changes["c"].value.as_deref(), Some("42"));
        assert_eq!(out.changes["b"].hidden, Some(true));
        assert_eq!(out.changes["a"].read_only, Some(true));
        assert_eq!(out.changes["total"].value.as_deref(), Some("54.5"));
        assert_eq!(out.alerts, vec!["Done".to_string()]);
        assert_eq!(
            out.blocked,
            vec!["submitForm".to_string(), "launchURL".to_string()]
        );
    }

    #[test]
    fn dates_check_boxes_and_reset() {
        let mut d = field("when", Kind::Text, "3-11-2026");
        d.scripts.format = "AFDate_FormatEx(\"d mmmm yyyy\");".into();
        let mut box_ = field("ok", Kind::Checkbox, "Off");
        box_.export_values = vec!["Yes".into()];
        let mut n = field("n", Kind::Text, "");
        n.scripts.calculate =
            "event.value = this.getField(\"ok\").isBoxChecked(0) ? util.printf(\"%,0.2f\", 1234.5) : \"no\";".into();
        let mut reset = field("reset", Kind::Button, "");
        reset.scripts.action = "this.resetForm();".into();
        let form = Form {
            fields: vec![d, box_, n, reset],
            order: vec!["n".into()],
            pages: 2,
            ..Form::default()
        };
        let out = run(
            &form,
            &Event::Commit {
                field: "ok".into(),
                value: "Yes".into(),
            },
            Limits::default(),
        )
        .unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.changes["n"].value.as_deref(), Some("1,234.50"));
        assert_eq!(
            out.changes["when"].display.as_deref(),
            Some("3 November 2026")
        );
        let out = run(
            &form,
            &Event::Click {
                field: "reset".into(),
            },
            Limits::default(),
        )
        .unwrap();
        assert_eq!(out.changes["when"].value.as_deref(), Some(""));
        assert_eq!(out.changes["n"].value.as_deref(), Some("no"));
    }

    #[test]
    fn errors_endless_loops_and_the_outside_world_are_contained() {
        let mut bad = field("bad", Kind::Text, "");
        bad.scripts.calculate = "event.value = nosuchthing.value;".into();
        let mut forever = field("forever", Kind::Text, "");
        forever.scripts.calculate = "while (true) {}".into();
        let mut probe = field("probe", Kind::Text, "");
        probe.scripts.calculate = "event.value = [typeof require, typeof process, typeof fetch, typeof XMLHttpRequest, typeof Deno].join(',');".into();
        let form = Form {
            fields: vec![bad, probe.clone()],
            order: vec!["bad".into(), "probe".into()],
            ..Form::default()
        };
        let out = run(&form, &Event::CalculateAll, Limits::default()).unwrap();
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert!(out.errors[0].starts_with("bad (calculate)"));
        assert_eq!(
            out.changes["probe"].value.as_deref(),
            Some("undefined,undefined,undefined,undefined,undefined")
        );
        let form = Form {
            fields: vec![forever, probe],
            order: vec!["forever".into(), "probe".into()],
            ..Form::default()
        };
        let small = Limits {
            loop_iterations: 10_000,
            ..Limits::default()
        };
        // The endless loop is stopped, and the run changes nothing.
        assert_eq!(
            run(&form, &Event::CalculateAll, small),
            Err(RunError::Limit)
        );
        let mut deep = field("deep", Kind::Text, "");
        deep.scripts.calculate = "function f(n) { return f(n + 1); } event.value = f(0);".into();
        let form = Form {
            fields: vec![deep],
            order: vec!["deep".into()],
            ..Form::default()
        };
        assert!(run(&form, &Event::CalculateAll, Limits::default()).is_err());
    }
}
