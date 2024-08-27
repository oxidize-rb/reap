#![recursion_limit = "1024"]

extern crate proc_macro;
extern crate quote;
extern crate syn;

use proc_macro::TokenStream;

#[proc_macro_attribute]
#[cfg(not(feature = "timed"))]
// no-op implementation of macro for when feature is disabled
pub fn timed(_: TokenStream, item: TokenStream) -> TokenStream {
    // Return the item as is when the feature is not enabled
    item
}

#[proc_macro_attribute]
#[cfg(feature = "timed")]
/// Macro for wrapping functions with timing.
///
/// ~Cargo-culted from https://github.com/Manishearth/rust-adorn/blob/master/src/lib.rs
pub fn timed(_: TokenStream, item: TokenStream) -> TokenStream {
    use quote::quote;
    use syn::{parse_macro_input, FnArg, ItemFn};

    let input = parse_macro_input!(item as ItemFn);
    let sig = &input.sig;

    if sig.generics.where_clause.is_some() {
        panic!("#[timed()] does not work with where clauses")
    }

    let mut args = vec![];
    for arg in sig.inputs.iter() {
        match *arg {
            FnArg::Typed(ref pat) => {
                args.push(quote!(#pat));
            }
            _ => panic!(),
        }
    }

    let funcname = &sig.ident;
    let generics = &sig.generics;
    let attributes = &input.attrs;
    let vis = &input.vis;
    let constness = &sig.constness;
    let unsafety = &sig.unsafety;
    let abi = &sig.abi;
    let output = &sig.output;
    let body = &input.block;
    let label = funcname.to_string();

    quote!(
        #(#attributes),*
        #vis #constness #unsafety #abi fn #funcname #generics (#(#args),*) #output {
            use std::time::{Duration, Instant};

            let start = Instant::now();
            let result = { #body };
            let elapsed = start.elapsed();
            println!("{}: {}.{}s", #label, elapsed.as_secs(), elapsed.subsec_millis());

            result
        }
    )
    .into()
}
