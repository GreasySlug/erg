use std::str::FromStr;

use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;

use erg_common::traits::Locational;
use erg_compiler::hir::{Accessor, Def, Dict, Expr, KeyValue, List, Set, Tuple};
use erg_compiler::varinfo::{AbsLocation, VarInfo};
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::symbol::symbol_kind;
use crate::util::NormalizedUrl;

fn hierarchy_item<C: BuildRunnable, P: Parsable>(
    server: &Server<C, P>,
    name: String,
    vi: &VarInfo,
) -> Option<CallHierarchyItem> {
    let loc = server.abs_loc_to_lsp_loc(&vi.def_loc)?;
    Some(CallHierarchyItem {
        name,
        kind: symbol_kind(vi),
        tags: None,
        detail: Some(vi.t.to_string()),
        uri: loc.uri,
        range: loc.range,
        selection_range: loc.range,
        data: Some(vi.def_loc.to_string().into()),
    })
}

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_call_hierarchy_incoming(
        &mut self,
        params: CallHierarchyIncomingCallsParams,
    ) -> ELSResult<Option<Vec<CallHierarchyIncomingCall>>> {
        let mut res = vec![];
        _log!(self, "call hierarchy incoming calls requested: {params:?}");
        let Some(data) = params.item.data.as_ref().and_then(|d| d.as_str()) else {
            return Ok(None);
        };
        let Ok(loc) = AbsLocation::from_str(data) else {
            return Ok(None);
        };
        if let Some(refs) = self.shared.index.get_refs(&loc) {
            for referrer_loc in refs.referrers.iter() {
                let Some(uri) = referrer_loc
                    .module
                    .as_ref()
                    .and_then(|path| NormalizedUrl::from_file_path(path).ok())
                else {
                    continue;
                };
                let Some(pos) = self.loc_to_pos(&uri, referrer_loc.loc) else {
                    continue;
                };
                if let Some(def) = self.get_min::<Def>(&uri, pos) {
                    if def.sig.is_subr() {
                        let Some(from) = hierarchy_item(
                            self,
                            def.sig.inspect().to_string(),
                            &def.sig.ident().vi,
                        ) else {
                            continue;
                        };
                        // the call site within the caller (`from`)
                        let from_ranges = self
                            .loc_to_range(&uri, referrer_loc.loc)
                            .into_iter()
                            .collect();
                        let call = CallHierarchyIncomingCall { from, from_ranges };
                        res.push(call);
                    }
                }
            }
        }
        Ok(Some(res))
    }

    pub(crate) fn handle_call_hierarchy_outgoing(
        &mut self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> ELSResult<Option<Vec<CallHierarchyOutgoingCall>>> {
        _log!(self, "call hierarchy outgoing calls requested: {params:?}");
        let Some(data) = params.item.data.as_ref().and_then(|d| d.as_str()) else {
            return Ok(None);
        };
        let Ok(loc) = AbsLocation::from_str(data) else {
            return Ok(None);
        };
        let Some(module) = loc.module else {
            return Ok(None);
        };
        let uri = NormalizedUrl::from_file_path(module)?;
        let Some(pos) = self.loc_to_pos(&uri, loc.loc) else {
            return Ok(None);
        };
        let mut calls = vec![];
        if let Some(def) = self.get_min::<Def>(&uri, pos) {
            for chunk in def.body.block.iter() {
                calls.extend(self.gen_outgoing_call(&uri, chunk));
            }
        }
        Ok(Some(calls))
    }

    /// Indirect calls are excluded. For example, calls in an anonymous function.
    fn gen_outgoing_call(
        &self,
        uri: &NormalizedUrl,
        expr: &Expr,
    ) -> Vec<CallHierarchyOutgoingCall> {
        let mut calls = vec![];
        match expr {
            Expr::Call(call) => {
                for arg in call.args.pos_args.iter() {
                    calls.extend(self.gen_outgoing_call(uri, &arg.expr));
                }
                if let Some(var) = call.args.var_args.as_ref() {
                    calls.extend(self.gen_outgoing_call(uri, &var.expr));
                }
                for arg in call.args.kw_args.iter() {
                    calls.extend(self.gen_outgoing_call(uri, &arg.expr));
                }
                if let Some(attr) = call.attr_name.as_ref() {
                    let Some(to) = hierarchy_item(self, attr.inspect().to_string(), &attr.vi)
                    else {
                        return calls;
                    };
                    // the call site (the callee name) within the current item
                    let from_ranges = self.loc_to_range(uri, attr.loc()).into_iter().collect();
                    calls.push(CallHierarchyOutgoingCall { to, from_ranges });
                } else if let Expr::Accessor(acc) = call.obj.as_ref() {
                    let Some(to) =
                        hierarchy_item(self, acc.last_name().to_string(), acc.var_info())
                    else {
                        return calls;
                    };
                    let from_ranges = self.loc_to_range(uri, acc.loc()).into_iter().collect();
                    calls.push(CallHierarchyOutgoingCall { to, from_ranges });
                }
                calls
            }
            Expr::TypeAsc(tasc) => self.gen_outgoing_call(uri, &tasc.expr),
            Expr::Accessor(Accessor::Attr(attr)) => self.gen_outgoing_call(uri, &attr.obj),
            Expr::BinOp(binop) => {
                calls.extend(self.gen_outgoing_call(uri, &binop.lhs));
                calls.extend(self.gen_outgoing_call(uri, &binop.rhs));
                calls
            }
            Expr::UnaryOp(unop) => self.gen_outgoing_call(uri, &unop.expr),
            Expr::List(List::Normal(lis)) => {
                for arg in lis.elems.pos_args.iter() {
                    calls.extend(self.gen_outgoing_call(uri, &arg.expr));
                }
                calls
            }
            Expr::Dict(Dict::Normal(dict)) => {
                for KeyValue { key, value } in dict.kvs.iter() {
                    calls.extend(self.gen_outgoing_call(uri, key));
                    calls.extend(self.gen_outgoing_call(uri, value));
                }
                calls
            }
            Expr::Set(Set::Normal(set)) => {
                for arg in set.elems.pos_args.iter() {
                    calls.extend(self.gen_outgoing_call(uri, &arg.expr));
                }
                calls
            }
            Expr::Tuple(Tuple::Normal(tuple)) => {
                for arg in tuple.elems.pos_args.iter() {
                    calls.extend(self.gen_outgoing_call(uri, &arg.expr));
                }
                calls
            }
            Expr::Record(rec) => {
                for attr in rec.attrs.iter() {
                    for chunk in attr.body.block.iter() {
                        calls.extend(self.gen_outgoing_call(uri, chunk));
                    }
                }
                calls
            }
            Expr::Def(def) if !def.sig.is_subr() => {
                for chunk in def.body.block.iter() {
                    calls.extend(self.gen_outgoing_call(uri, chunk));
                }
                calls
            }
            _ => calls,
        }
    }

    pub(crate) fn handle_call_hierarchy_prepare(
        &mut self,
        params: CallHierarchyPrepareParams,
    ) -> ELSResult<Option<Vec<CallHierarchyItem>>> {
        _log!(self, "call hierarchy prepare requested: {params:?}");
        let mut res = vec![];
        let uri = NormalizedUrl::new(params.text_document_position_params.text_document.uri);
        let pos = params.text_document_position_params.position;
        if let Some(token) = self.file_cache.get_symbol(&uri, pos) {
            if let Some(vi) = self.get_definition(&uri, &token)? {
                let Some(item) = hierarchy_item(self, token.content.to_string(), &vi) else {
                    return Ok(None);
                };
                res.push(item);
            }
        }
        Ok(Some(res))
    }
}
