#![feature(rustc_private)]
extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_span;
use rustc_ast::{
    self as ast,
    visit::{self, Visitor},
};
use rustc_driver::{Callbacks, Compilation};
fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}
struct Items<'a> {
    source: &'a rustc_span::source_map::SourceMap,
    path: Vec<String>,
}
impl<'ast> Visitor<'ast> for Items<'_> {
    fn visit_item(&mut self, item: &'ast ast::Item) {
        let name = item
            .kind
            .ident()
            .map(|i| i.name.to_string())
            .unwrap_or_else(|| format!("@{}", item.span.lo().0));
        self.path.push(name);
        let kind = match &item.kind {
            ast::ItemKind::Fn(_) => "fn",
            ast::ItemKind::Mod(..) => "mod",
            ast::ItemKind::MacCall(_) | ast::ItemKind::MacroDef(..) => "macro_opaque",
            _ => "other",
        };
        if kind != "other" {
            let span = format!("{:?}", item.span);
            eprintln!(
                "PARSE_ITEM\t{}\t{}\t{}\t{}\t{}",
                kind,
                hex(&self.path.join("::")),
                hex(&span),
                self.source
                    .lookup_source_file(item.span.lo())
                    .original_relative_byte_pos(item.span.lo())
                    .0,
                self.source
                    .lookup_source_file(item.span.hi())
                    .original_relative_byte_pos(item.span.hi())
                    .0
            );
            for attr in &item.attrs {
                for name in ["ktest", "test"] {
                    if kind == "fn" && attr.has_name(rustc_span::Symbol::intern(name)) {
                        eprintln!("PARSE_TEST\t{}\t{}", hex(&span), name);
                    }
                }
                let role = ["ktest", "test", "should_panic", "ignore", "cfg", "cfg_attr"]
                    .iter()
                    .copied()
                    .find(|name| attr.has_name(rustc_span::Symbol::intern(name)))
                    .unwrap_or("other");
                let text = self.source.span_to_snippet(attr.span).unwrap();
                eprintln!(
                    "PARSE_ATTR\t{}\t{}\t{}\t{}\t{}\t{}",
                    hex(&span),
                    hex(&format!("{:?}", attr.span)),
                    role,
                    self.source
                        .lookup_source_file(attr.span.lo())
                        .original_relative_byte_pos(attr.span.lo())
                        .0,
                    self.source
                        .lookup_source_file(attr.span.hi())
                        .original_relative_byte_pos(attr.span.hi())
                        .0,
                    hex(&text)
                );
            }
            if let ast::ItemKind::Fn(f) = &item.kind {
                if let Some(body) = &f.body {
                    let text = self.source.span_to_snippet(body.span).unwrap();
                    eprintln!(
                        "PARSE_BODY\t{}\t{}\t{}\t{}\t{}",
                        hex(&span),
                        hex(&format!("{:?}", body.span)),
                        self.source
                            .lookup_source_file(body.span.lo())
                            .original_relative_byte_pos(body.span.lo())
                            .0,
                        self.source
                            .lookup_source_file(body.span.hi())
                            .original_relative_byte_pos(body.span.hi())
                            .0,
                        hex(&text)
                    );
                }
            }
        }
        visit::walk_item(self, item);
        self.path.pop();
    }
}
struct Probe;
impl Callbacks for Probe {
    fn after_crate_root_parsing(
        &mut self,
        compiler: &rustc_interface::interface::Compiler,
        krate: &mut ast::Crate,
    ) -> Compilation {
        let mut items = Items {
            source: compiler.sess.source_map(),
            path: Vec::new(),
        };
        for item in &krate.items {
            items.visit_item(item);
        }
        eprintln!("PARSE_COMPLETE");
        Compilation::Stop
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    rustc_driver::run_compiler(&args, &mut Probe);
}
