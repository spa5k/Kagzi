//! Implementation of the `kagzi_workflow!` macro (Variant 5).
//!
//! This macro provides a block-based syntax for defining workflows with:
//! - Simple `run!` macro for step execution
//! - Clear variable flow between steps
//! - Rust-like syntax with minimal magic

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::ItemFn;

/// Expands the `kagzi_workflow!` macro.
pub fn expand_kagzi_workflow(input: TokenStream) -> TokenStream {
    let input = proc_macro2::TokenStream::from(input);

    // Try to parse as an ItemFn
    let func: ItemFn = match syn::parse2(input) {
        Ok(func) => func,
        Err(e) => return e.to_compile_error().into(),
    };

    // Generate the enhanced function with run! macro
    let enhanced = generate_enhanced_workflow(func);

    enhanced.into()
}

/// Generates an enhanced workflow function with built-in run! macro.
fn generate_enhanced_workflow(func: ItemFn) -> TokenStream2 {
    let fn_attrs = func.attrs;
    let fn_vis = func.vis;
    let fn_sig = func.sig;
    let fn_block = func.block;

    // Extract the function name for tracing
    let fn_name = &fn_sig.ident;

    // Generate the enhanced function with run! macro
    quote! {
        #(#fn_attrs)*
        #fn_vis #fn_sig {
            // Define the run! macro for this workflow
            macro_rules! run {
                ($step_name:expr, $step_expr:expr) => {
                    {
                        ctx.run($step_name, $step_expr).await?
                    }
                };
            }

            // Add workflow-level tracing
            let span = tracing::info_span!(
                "workflow",
                workflow = stringify!(#fn_name),
            );
            let _enter = span.enter();

            tracing::info!(
                "Starting workflow: {}",
                stringify!(#fn_name)
            );

            // The original function body
            #fn_block
        }
    }
}
