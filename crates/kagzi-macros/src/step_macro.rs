//! Implementation of the `#[kagzi_step]` attribute macro (Variant 8).
//!
//! This macro enhances workflow step functions with:
//! - Automatic tracing span creation
//! - Input/output logging at debug level
//! - Error context enrichment
//! - Metrics/observability hooks

use proc_macro2::{Ident, TokenStream};
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{Attribute, Error, FnArg, ItemFn, ReturnType, Signature, Type, TypePath};

/// Expands the `#[kagzi_step]` attribute macro.
///
/// The macro adds observability features to workflow step functions without
/// changing the function signature or behavior.
pub fn expand_kagzi_step(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = match syn::parse2::<ItemFn>(item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error(),
    };

    // Validate the function signature
    if let Err(e) = validate_function_signature(&input_fn) {
        return e.to_compile_error();
    }

    // Extract function information
    let fn_name = &input_fn.sig.ident;
    let fn_attrs = &input_fn.attrs;
    let fn_vis = &input_fn.vis;
    let fn_docs = extract_docs(&input_fn.attrs);

    // Parse output type
    let output_type = match extract_output_type(&input_fn.sig) {
        Ok(result) => result,
        Err(e) => return e.to_compile_error(),
    };

    // Generate the enhanced function
    generate_step_enhancement(StepEnhancementConfig {
        docs: fn_docs,
        attrs: fn_attrs,
        vis: fn_vis,
        fn_name,
        input_params: &input_fn.sig.inputs,
        output_type,
        block: &input_fn.block,
    })
}

/// Validates that the function has the expected signature.
fn validate_function_signature(func: &ItemFn) -> Result<(), Error> {
    let sig = &func.sig;

    // Must be async
    if sig.asyncness.is_none() {
        return Err(Error::new_spanned(
            sig.fn_token,
            "kagzi_step functions must be async",
        ));
    }

    // Check number of parameters (must have 1 or 2 parameters)
    let param_count = sig.inputs.len();
    if param_count < 1 || param_count > 2 {
        return Err(Error::new_spanned(
            sig.inputs.clone(),
            "kagzi_step functions must have 1 or 2 parameters: optionally a WorkflowContext as first parameter and an input parameter",
        ));
    }

    // Must return a Result
    match &sig.output {
        ReturnType::Type(_, ty) => {
            if !is_result_type(ty) {
                return Err(Error::new_spanned(
                    ty,
                    "kagzi_step functions must return a Result<T, E>",
                ));
            }
        }
        ReturnType::Default => {
            return Err(Error::new_spanned(
                sig.fn_token,
                "kagzi_step functions must have an explicit return type",
            ));
        }
    }

    Ok(())
}

/// Checks if a type is a Result<T, E>.
fn is_result_type(ty: &Type) -> bool {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            if let Some(segment) = path.segments.last() {
                segment.ident == "Result"
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Extracts the output type from function signature.
fn extract_output_type(sig: &Signature) -> Result<Type, Error> {
    match &sig.output {
        ReturnType::Type(_, ty) => {
            // Extract the Ok type from Result<T, E>
            match &**ty {
                Type::Path(TypePath { path, .. }) => {
                    if let Some(segment) = path.segments.last()
                        && segment.ident == "Result"
                    {
                        use syn::{GenericArgument, PathArguments};
                        if let PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(GenericArgument::Type(t)) = args.args.first()
                        {
                            return Ok(t.clone());
                        }
                    }
                    Err(Error::new_spanned(ty, "Expected Result<T, E> type"))
                }
                _ => Err(Error::new_spanned(ty, "Expected Result<T, E> type")),
            }
        }
        ReturnType::Default => Err(Error::new_spanned(
            sig.fn_token,
            "Function must have a return type",
        )),
    }
}

/// Extracts documentation comments from function attributes.
fn extract_docs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .cloned()
        .collect()
}

/// Configuration for generating step enhancements
struct StepEnhancementConfig<'a> {
    docs: Vec<Attribute>,
    attrs: &'a [Attribute],
    vis: &'a syn::Visibility,
    fn_name: &'a Ident,
    input_params: &'a Punctuated<FnArg, Comma>,
    output_type: Type,
    block: &'a syn::Block,
}

/// Generates the enhanced step function with observability.
fn generate_step_enhancement(config: StepEnhancementConfig) -> TokenStream {
    let StepEnhancementConfig {
        docs,
        attrs,
        vis,
        fn_name,
        input_params,
        output_type,
        block,
    } = config;

    quote::quote! {
        #(#attrs)*
        #(#docs)*
        #vis async fn #fn_name(
            #input_params
        ) -> anyhow::Result<#output_type> {
            use anyhow::Context;
            use tracing::Instrument;

            let span = tracing::info_span!(
                "step",
                step = stringify!(#fn_name),
            );

            async {
                tracing::debug!(
                    "Starting step: {}",
                    stringify!(#fn_name)
                );

                // Execute the function body with error context
                let result: anyhow::Result<#output_type> = #block;
                match result {
                    Ok(output) => {
                        tracing::debug!(
                            output = ?output,
                            "Step completed successfully: {}",
                            stringify!(#fn_name)
                        );
                        Ok(output)
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "Step failed: {}",
                            stringify!(#fn_name)
                        );
                        Err(e.context(format!("Step '{}' failed", stringify!(#fn_name))))
                    }
                }
            }.instrument(span).await
        }
    }
}
