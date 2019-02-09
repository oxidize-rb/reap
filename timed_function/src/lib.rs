#![recursion_limit = "1024"]

extern crate proc_macro;
extern crate quote;
extern crate syn;

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input,
    FnArg,
    ItemFn,
};

#[proc_macro_attribute]
/// Macro for wrapping functions with timing.
///
/// ~Cargo-culted from https://github.com/Manishearth/rust-adorn/blob/master/src/lib.rs
pub fn timed(_: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    if input.decl.generics.where_clause.is_some() {
        panic!("#[timed()] does not work with where clauses")
    }

    let mut args = vec![];
    for arg in input.decl.inputs.iter() {
        match *arg {
            FnArg::Captured(ref cap) => {
                let ty = &cap.ty;
                let pat = &cap.pat;
                args.push(quote!(#pat: #ty));
            }
             _ => panic!()
        }
    }

    let funcname = &input.ident;
    let attributes = &input.attrs;
    let vis = &input.vis;
    let constness = &input.constness;
    let unsafety = &input.unsafety;
    let abi = &input.abi;
    let output = &input.decl.output;
    let body = &input.block;
    let label = funcname.to_string();

    quote!(
        #(#attributes),*
        #vis #constness #unsafety #abi fn #funcname (#(#args),*) #output {
            use std::time::{Duration, Instant};

            let start = Instant::now();
            let result = { #body };
            let elapsed = start.elapsed();
            println!("{}: {}.{}s", #label, elapsed.as_secs(), elapsed.subsec_millis());

            result
        }
    ).into()
}

