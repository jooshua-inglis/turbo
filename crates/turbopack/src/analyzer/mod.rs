use std::{fmt::Display, future::Future, mem::take};

use crate::ecmascript::utils::lit_to_string;

pub(crate) use self::imports::ImportMap;
use swc_atoms::{js_word, JsWord};
use swc_common::{collections::AHashSet, Mark};
use swc_ecmascript::{ast::*, utils::ident::IdentLike};
use url::Url;

pub mod builtin;
pub mod graph;
mod imports;
pub mod linker;
pub mod well_known;

/// TODO: Use `Arc`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsValue {
    /// Denotes a single string literal, which does not have any unknown value.
    ///
    /// TODO: Use a type without span
    Constant(Lit),

    Array(Vec<JsValue>),

    Url(Url),

    Alternatives(Vec<JsValue>),

    // TODO no predefined kinds, only JsWord
    FreeVar(FreeVarKind),

    Variable(Id),

    /// `foo.${unknownVar}.js` => 'foo' + Unknown + '.js'
    Concat(Vec<JsValue>),

    /// This can be converted to [JsValue::Concat] if the type of the variable
    /// is string.
    Add(Vec<JsValue>),

    /// `(callee, args)`
    Call(Box<JsValue>, Vec<JsValue>),

    /// `obj[prop]`
    Member(Box<JsValue>, Box<JsValue>),

    /// This is a reference to a imported module
    Module(JsWord),

    /// Some kind of well known object
    WellKnownObject(WellKnownObjectKind),

    /// Some kind of well known function
    WellKnownFunction(WellKnownFunctionKind),

    /// Not analyzable.
    Unknown(Option<Box<JsValue>>, &'static str),

    /// `(return_value)`
    Function(Box<JsValue>),

    Argument(usize),
}

impl From<&'_ str> for JsValue {
    fn from(v: &str) -> Self {
        Str::from(v).into()
    }
}

impl From<String> for JsValue {
    fn from(v: String) -> Self {
        Str::from(v).into()
    }
}

impl From<Str> for JsValue {
    fn from(v: Str) -> Self {
        Lit::Str(v).into()
    }
}

impl From<Lit> for JsValue {
    fn from(v: Lit) -> Self {
        JsValue::Constant(v)
    }
}

impl Default for JsValue {
    fn default() -> Self {
        JsValue::Unknown(None, "")
    }
}

impl Display for JsValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsValue::Constant(lit) => write!(f, "{}", lit_to_string(lit)),
            JsValue::Url(url) => write!(f, "{}", url),
            JsValue::Array(elems) => write!(
                f,
                "[{}]",
                elems
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(". ")
            ),
            JsValue::Alternatives(list) => write!(
                f,
                "({})",
                list.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            JsValue::FreeVar(name) => write!(f, "FreeVar({:?})", name),
            JsValue::Variable(name) => write!(f, "Variable({}#{:?})", name.0, name.1),
            JsValue::Concat(list) => write!(
                f,
                "`{}`",
                list.iter()
                    .map(|v| match v {
                        JsValue::Constant(Lit::Str(str)) => str.value.to_string(),
                        _ => format!("${{{}}}", v),
                    })
                    .collect::<Vec<_>>()
                    .join("")
            ),
            JsValue::Add(list) => write!(
                f,
                "({})",
                list.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
            JsValue::Call(callee, list) => write!(
                f,
                "{}({})",
                callee,
                list.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            JsValue::Member(obj, prop) => write!(f, "{}[{}]", obj, prop),
            JsValue::Module(name) => write!(f, "Module({})", name),
            JsValue::Unknown(..) => write!(f, "???"),
            JsValue::WellKnownObject(obj) => write!(f, "WellKnownObject({:?})", obj),
            JsValue::WellKnownFunction(func) => write!(f, "WellKnownFunction({:?})", func),
            JsValue::Function(return_value) => write!(f, "Function(return = {:?})", return_value),
            JsValue::Argument(index) => write!(f, "Argument({})", index),
        }
    }
}

impl JsValue {
    pub fn explain_args(args: &Vec<JsValue>, depth: usize) -> (String, String) {
        let mut hints = Vec::new();
        let explainer = args
            .iter()
            .map(|arg| arg.explain_internal(&mut hints, depth))
            .collect::<Vec<_>>()
            .join(", ");
        (
            explainer,
            hints
                .into_iter()
                .map(|h| format!("\n{h}"))
                .collect::<String>(),
        )
    }

    pub fn explain(&self, depth: usize) -> (String, String) {
        let mut hints = Vec::new();
        let explainer = self.explain_internal(&mut hints, depth);
        (
            explainer,
            hints
                .into_iter()
                .map(|h| format!("\n{h}"))
                .collect::<String>(),
        )
    }

    fn explain_internal(&self, hints: &mut Vec<String>, depth: usize) -> String {
        match self {
            JsValue::Constant(lit) => format!("{}", lit_to_string(lit)),
            JsValue::Array(elems) => format!(
                "[{}]",
                elems
                    .iter()
                    .map(|v| v.explain_internal(hints, depth))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            JsValue::Url(url) => format!("{}", url),
            JsValue::Alternatives(list) => format!(
                "({})",
                list.iter()
                    .map(|v| v.explain_internal(hints, depth))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            JsValue::FreeVar(name) => format!("FreeVar({:?})", name),
            JsValue::Variable(name) => {
                format!("{}", name.0)
            }
            JsValue::Argument(index) => {
                format!("Argument({})", index)
            }
            JsValue::Concat(list) => format!(
                "`{}`",
                list.iter()
                    .map(|v| match v {
                        JsValue::Constant(Lit::Str(str)) => str.value.to_string(),
                        _ => format!("${{{}}}", v.explain_internal(hints, depth)),
                    })
                    .collect::<Vec<_>>()
                    .join("")
            ),
            JsValue::Add(list) => format!(
                "({})",
                list.iter()
                    .map(|v| v.explain_internal(hints, depth))
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
            JsValue::Call(callee, list) => format!(
                "{}({})",
                callee.explain_internal(hints, depth),
                list.iter()
                    .map(|v| v.explain_internal(hints, depth))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            JsValue::Member(obj, prop) => {
                format!(
                    "{}[{}]",
                    obj.explain_internal(hints, depth),
                    prop.explain_internal(hints, depth)
                )
            }
            JsValue::Module(name) => {
                format!("module<{}>", name)
            }
            JsValue::Unknown(inner, explainer) => {
                if depth == 0 || explainer.is_empty() {
                    format!("???")
                } else if let Some(inner) = inner {
                    let i = hints.len();
                    hints.push(String::new());
                    hints[i] = format!(
                        "- *{}* {}\n  ⚠️  {}",
                        i,
                        inner.explain_internal(hints, depth - 1),
                        explainer,
                    );
                    format!("*{}*", i)
                } else {
                    let i = hints.len();
                    hints.push(String::new());
                    hints[i] = format!("- *{}* {}", i, explainer);
                    format!("*{}*", i)
                }
            }
            JsValue::WellKnownObject(obj) => {
                let (name, explainer) = match obj {
                    WellKnownObjectKind::PathModule => (
                        "path",
                        "The Node.js path module: https://nodejs.org/api/path.html",
                    ),
                    WellKnownObjectKind::FsModule => (
                        "fs",
                        "The Node.js fs module: https://nodejs.org/api/fs.html",
                    ),
                    WellKnownObjectKind::UrlModule => (
                        "url",
                        "The Node.js url module: https://nodejs.org/api/url.html",
                    ),
                    WellKnownObjectKind::ChildProcess => (
                        "child_process",
                        "The Node.js child_process module: https://nodejs.org/api/child_process.html",
                    ),
                };
                if depth > 0 {
                    let i = hints.len();
                    hints.push(format!("- *{i}* {name}: {explainer}"));
                    format!("{name}*{i}*")
                } else {
                    name.to_string()
                }
            }
            JsValue::WellKnownFunction(func) => {
                let (name, explainer) = match func {
                    WellKnownFunctionKind::PathJoin => (
                        format!("path.join"),
                        "The Node.js path.join method: https://nodejs.org/api/path.html#pathjoinpaths",
                    ),
                    WellKnownFunctionKind::Import => (
                        format!("import"),
                        "The dynamic import() method from the ESM specification: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/import#dynamic_imports"
                    ),
                    WellKnownFunctionKind::Require => (format!("require"), "The require method from CommonJS"),
                    WellKnownFunctionKind::RequireResolve => (format!("require.resolve"), "The require.resolve method from CommonJS"),
                    WellKnownFunctionKind::FsReadMethod(name) => (
                        format!("fs.{name}"),
                        "A file reading method from the Node.js fs module: https://nodejs.org/api/fs.html",
                    ),
                    WellKnownFunctionKind::PathToFileUrl => (
                        format!("url.pathToFileURL"),
                        "The Node.js url.pathToFileURL method: https://nodejs.org/api/url.html#urlpathtofileurlpath",
                    ),
                    WellKnownFunctionKind::ChildProcessSpawn => (
                        format!("child_process.spawn"),
                        "The Node.js child_process.spawn method: https://nodejs.org/api/child_process.html#child_processspawncommand-args-options",
                    ),
                };
                if depth > 0 {
                    let i = hints.len();
                    hints.push(format!("- *{i}* {name}: {explainer}"));
                    format!("{name}s*{i}*")
                } else {
                    name
                }
            }
            JsValue::Function(return_value) => {
                format!("A function which returns ({:?})", return_value)
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let JsValue::Constant(Lit::Str(str)) = self {
            Some(&*str.value)
        } else {
            None
        }
    }

    pub fn as_word(&self) -> Option<&JsWord> {
        if let JsValue::Constant(Lit::Str(str)) = self {
            Some(&str.value)
        } else {
            None
        }
    }

    pub async fn visit_async<'a, F, R, E>(self, visitor: &mut F) -> Result<(Self, bool), E>
    where
        R: 'a + Future<Output = Result<(Self, bool), E>>,
        F: 'a + FnMut(JsValue) -> R,
    {
        let (v, modified) = self.for_each_children_async(visitor).await?;
        let (v, m) = visitor(v).await?;
        if m {
            Ok((v, true))
        } else {
            Ok((v, modified))
        }
    }

    pub async fn for_each_children_async<'a, F, R, E>(
        mut self,
        visitor: &mut F,
    ) -> Result<(Self, bool), E>
    where
        R: 'a + Future<Output = Result<(Self, bool), E>>,
        F: 'a + FnMut(JsValue) -> R,
    {
        Ok(match &mut self {
            JsValue::Alternatives(list)
            | JsValue::Concat(list)
            | JsValue::Add(list)
            | JsValue::Array(list) => {
                let mut modified = false;
                for item in list.iter_mut() {
                    let (v, m) = visitor(take(item)).await?;
                    *item = v;
                    if m {
                        modified = true
                    }
                }
                (self, modified)
            }
            JsValue::Call(box callee, list) => {
                let (new_callee, mut modified) = visitor(take(callee)).await?;
                *callee = new_callee;
                for item in list.iter_mut() {
                    let (v, m) = visitor(take(item)).await?;
                    *item = v;
                    if m {
                        modified = true
                    }
                }
                (self, modified)
            }

            JsValue::Function(box return_value) => {
                let (new_return_value, modified) = visitor(take(return_value)).await?;
                *return_value = new_return_value;

                (self, modified)
            }
            JsValue::Member(box obj, box prop) => {
                let (v, m1) = visitor(take(obj)).await?;
                *obj = v;
                let (v, m2) = visitor(take(prop)).await?;
                *prop = v;
                (self, m1 || m2)
            }
            JsValue::Constant(_)
            | JsValue::FreeVar(_)
            | JsValue::Variable(_)
            | JsValue::Module(_)
            | JsValue::Url(_)
            | JsValue::WellKnownObject(_)
            | JsValue::WellKnownFunction(_)
            | JsValue::Unknown(..)
            | JsValue::Argument(..) => (self, false),
        })
    }

    pub fn visit_mut(&mut self, visitor: &mut impl FnMut(&mut JsValue) -> bool) -> bool {
        let modified = self.for_each_children_mut(visitor);
        if visitor(self) {
            true
        } else {
            modified
        }
    }

    pub fn visit_mut_recursive(&mut self, visitor: &mut impl FnMut(&mut JsValue) -> bool) -> bool {
        let modified = self.for_each_children_mut(&mut |value| {
            let m1 = value.visit_mut(visitor);
            let m2 = visitor(value);

            m1 || m2
        });
        if visitor(self) {
            true
        } else {
            modified
        }
    }

    pub fn for_each_children_mut(
        &mut self,
        visitor: &mut impl FnMut(&mut JsValue) -> bool,
    ) -> bool {
        match self {
            JsValue::Alternatives(list)
            | JsValue::Concat(list)
            | JsValue::Add(list)
            | JsValue::Array(list) => {
                let mut modified = false;
                for item in list.iter_mut() {
                    if visitor(item) {
                        modified = true
                    }
                }
                modified
            }
            JsValue::Call(callee, list) => {
                let mut modified = visitor(callee);
                for item in list.iter_mut() {
                    if visitor(item) {
                        modified = true
                    }
                }
                modified
            }
            JsValue::Function(return_value) => {
                let modified = visitor(return_value);

                modified
            }
            JsValue::Member(obj, prop) => {
                let modified = visitor(obj);
                visitor(prop) || modified
            }
            JsValue::Constant(_)
            | JsValue::FreeVar(_)
            | JsValue::Variable(_)
            | JsValue::Module(_)
            | JsValue::Url(_)
            | JsValue::WellKnownObject(_)
            | JsValue::WellKnownFunction(_)
            | JsValue::Unknown(..)
            | JsValue::Argument(..) => false,
        }
    }

    pub fn visit(&mut self, visitor: &mut impl FnMut(&JsValue)) {
        self.for_each_children(visitor);
        visitor(self);
    }

    pub fn for_each_children(&self, visitor: &mut impl FnMut(&JsValue)) {
        match self {
            JsValue::Alternatives(list)
            | JsValue::Concat(list)
            | JsValue::Add(list)
            | JsValue::Array(list) => {
                for item in list.iter() {
                    visitor(item);
                }
            }
            JsValue::Call(callee, list) => {
                visitor(callee);
                for item in list.iter() {
                    visitor(item);
                }
            }
            JsValue::Function(return_value) => {
                visitor(return_value);
            }
            JsValue::Member(obj, prop) => {
                visitor(obj);
                visitor(prop);
            }
            JsValue::Constant(_)
            | JsValue::FreeVar(_)
            | JsValue::Variable(_)
            | JsValue::Module(_)
            | JsValue::Url(_)
            | JsValue::WellKnownObject(_)
            | JsValue::WellKnownFunction(_)
            | JsValue::Unknown(..)
            | JsValue::Argument(..) => {}
        }
    }

    pub fn is_string(&self) -> bool {
        match self {
            JsValue::Constant(Lit::Str(..)) | JsValue::Concat(_) => true,

            JsValue::Constant(..)
            | JsValue::Array(..)
            | JsValue::Url(..)
            | JsValue::Module(..)
            | JsValue::Function(..) => false,

            JsValue::FreeVar(FreeVarKind::Dirname) => true,
            JsValue::FreeVar(
                FreeVarKind::Require | FreeVarKind::Import | FreeVarKind::RequireResolve,
            ) => false,
            JsValue::FreeVar(FreeVarKind::Other(_)) => false,

            JsValue::Add(v) => v.iter().any(|v| v.is_string()),

            JsValue::Alternatives(v) => v.iter().all(|v| v.is_string()),

            JsValue::Variable(_) | JsValue::Unknown(..) | JsValue::Argument(..) => false,

            JsValue::Call(box JsValue::FreeVar(FreeVarKind::RequireResolve), _) => true,
            JsValue::Call(..) | JsValue::Member(..) => false,
            JsValue::WellKnownObject(_) | JsValue::WellKnownFunction(_) => false,
        }
    }

    fn add_alt(&mut self, v: Self) {
        if self == &v {
            return;
        }

        if let JsValue::Alternatives(list) = self {
            if !list.contains(&v) {
                list.push(v)
            }
        } else {
            let l = take(self);
            *self = JsValue::Alternatives(vec![l, v]);
        }
    }

    pub fn normalize_shallow(&mut self) {
        // TODO really doing shallow
        self.normalize();
    }

    pub fn normalize(&mut self) {
        self.for_each_children_mut(&mut |child| {
            child.normalize();
            true
        });
        // Handle nested
        match self {
            JsValue::Alternatives(v) => {
                let mut new = vec![];
                for v in take(v) {
                    match v {
                        JsValue::Alternatives(v) => new.extend(v),
                        v => new.push(v),
                    }
                }
                *v = new;
            }
            JsValue::Concat(v) => {
                // Remove empty strings
                v.retain(|v| match v {
                    JsValue::Constant(Lit::Str(Str {
                        value: js_word!(""),
                        ..
                    })) => false,
                    _ => true,
                });

                // TODO(kdy1): Remove duplicate
                let mut new = vec![];
                for v in take(v) {
                    match v {
                        JsValue::Concat(v) => new.extend(v),
                        JsValue::Constant(Lit::Str(ref str)) => {
                            if let Some(JsValue::Constant(Lit::Str(last))) = new.last_mut() {
                                *last = [&*last.value, &*str.value].concat().into();
                            } else {
                                new.push(v);
                            }
                        }
                        v => new.push(v),
                    }
                }
                if new.len() == 1 {
                    *self = new.into_iter().next().unwrap();
                } else {
                    *v = new;
                }
            }
            JsValue::Add(v) => {
                let mut added: Vec<JsValue> = Vec::new();
                let mut iter = take(v).into_iter();
                while let Some(item) = iter.next() {
                    if item.is_string() {
                        let mut concat = match added.len() {
                            0 => Vec::new(),
                            1 => vec![added.into_iter().next().unwrap()],
                            _ => vec![JsValue::Add(added)],
                        };
                        concat.push(item);
                        while let Some(item) = iter.next() {
                            concat.push(item);
                        }
                        *self = JsValue::Concat(concat);
                        return;
                    } else {
                        added.push(item);
                    }
                }
                *v = added;
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreeVarKind {
    /// `__dirname`
    Dirname,

    /// A reference to global `require`
    Require,

    /// A reference to `import`
    Import,

    /// A reference to global `require.resolve`
    RequireResolve,

    /// `abc` `some_global`
    Other(JsWord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WellKnownObjectKind {
    PathModule,
    FsModule,
    UrlModule,
    ChildProcess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WellKnownFunctionKind {
    PathJoin,
    Import,
    Require,
    RequireResolve,
    FsReadMethod(JsWord),
    PathToFileUrl,
    ChildProcessSpawn,
}

/// TODO(kdy1): Remove this once resolver distinguish between top-level bindings
/// and unresolved references https://github.com/swc-project/swc/issues/2956
///
/// Once the swc issue is resolved, it means we can know unresolved references
/// just by comparing [Mark]
fn is_unresolved(i: &Ident, bindings: &AHashSet<Id>, top_level_mark: Mark) -> bool {
    // resolver resolved `i` to non-top-level binding
    if i.span.ctxt.outer() != top_level_mark {
        return false;
    }

    // Check if there's a top level binding for `i`.
    // If it exists, `i` is reference to the binding.
    !bindings.contains(&i.to_id())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex};

    use anyhow::Result;
    use async_std::task::block_on;
    use swc_common::Mark;
    use swc_ecma_transforms_base::resolver::resolver_with_mark;
    use swc_ecmascript::{ast::EsVersion, parser::parse_file_as_module, visit::VisitMutWith};
    use testing::NormalizedOutput;

    use crate::{analyzer::builtin::replace_builtin, ecmascript::utils::lit_to_string};

    use super::{
        graph::{create_graph, EvalContext},
        linker::{link, LinkCache},
        well_known::replace_well_known,
        FreeVarKind, JsValue, WellKnownFunctionKind, WellKnownObjectKind,
    };

    #[testing::fixture("tests/analyzer/graph/**/input.js")]
    fn fixture(input: PathBuf) {
        let graph_snapshot_path = input.with_file_name("graph.snapshot");
        let resolved_snapshot_path = input.with_file_name("resolved.snapshot");

        testing::run_test(false, |cm, handler| {
            let fm = cm.load_file(&input).unwrap();

            let mut m = parse_file_as_module(
                &fm,
                Default::default(),
                EsVersion::latest(),
                None,
                &mut vec![],
            )
            .map_err(|err| err.into_diagnostic(&handler).emit())?;

            let top_level_mark = Mark::fresh(Mark::root());
            m.visit_mut_with(&mut resolver_with_mark(top_level_mark));

            let eval_context = EvalContext::new(&m, top_level_mark);

            let var_graph = create_graph(&m, &eval_context);

            {
                // Dump snapshot of graph

                let mut dump = var_graph.values.clone().into_iter().collect::<Vec<_>>();
                dump.sort_by(|a, b| a.0 .1.cmp(&b.0 .1));
                dump.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));

                NormalizedOutput::from(format!("{:#?}", dump))
                    .compare_to_file(&graph_snapshot_path)
                    .unwrap();
            }

            {
                // Dump snapshot of resolved

                let mut resolved = vec![];

                async fn visitor(v: JsValue) -> Result<(JsValue, bool)> {
                    Ok((
                        match v {
                            JsValue::Call(
                                box JsValue::WellKnownFunction(
                                    WellKnownFunctionKind::RequireResolve,
                                ),
                                ref args,
                            ) => match &args[0] {
                                JsValue::Constant(lit) => {
                                    JsValue::Constant((lit_to_string(&lit) + " (resolved)").into())
                                }
                                _ => JsValue::Unknown(Some(box v), "resolve.resolve non constant"),
                            },
                            JsValue::FreeVar(FreeVarKind::Require) => {
                                JsValue::WellKnownFunction(WellKnownFunctionKind::Require)
                            }
                            JsValue::FreeVar(FreeVarKind::Dirname) => {
                                JsValue::Constant("__dirname".into())
                            }
                            JsValue::FreeVar(kind) => {
                                JsValue::Unknown(Some(box JsValue::FreeVar(kind)), "unknown global")
                            }
                            JsValue::Module(ref name) => match &**name {
                                "path" => JsValue::WellKnownObject(WellKnownObjectKind::PathModule),
                                _ => return Ok((v, false)),
                            },
                            _ => {
                                let (v, m1) = replace_well_known(v);
                                let (v, m2) = replace_builtin(v);
                                return Ok((v, m1 || m2));
                            }
                        },
                        true,
                    ))
                }

                for ((id, ctx), val) in var_graph.values.iter() {
                    let val = val.clone();
                    let mut res = block_on(link(
                        &var_graph,
                        val,
                        &(|val| Box::pin(visitor(val))),
                        &Mutex::new(LinkCache::new()),
                    ))
                    .unwrap();
                    res.normalize();

                    let unique = var_graph.values.keys().filter(|(i, _)| id == i).count() == 1;
                    if unique {
                        resolved.push((id.to_string(), res));
                    } else {
                        resolved.push((format!("{id}{ctx:?}"), res));
                    }
                }
                resolved.sort_by(|a, b| a.0.cmp(&b.0));

                NormalizedOutput::from(format!("{:#?}", resolved))
                    .compare_to_file(&resolved_snapshot_path)
                    .unwrap();
            }

            Ok(())
        })
        .unwrap();
    }
}