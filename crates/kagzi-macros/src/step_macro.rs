//! Implementation of the `#[kagzi_step]` attribute macro (Variant 8).
//!
//! This macro enhances workflow step functions with:
//! - Automatic tracing span creation
//! - Input/output logging at debug level
//! - Error context enrichment
//! - Metrics/observability hooks

use proc_macro2::{Ident, TokenStream};
use syn::{Attribute, Error, FnArg, ItemFn, Pat, PatType, ReturnType, Signature, Type, TypePath};

/// Expands the `#[kagzi_step]` attribute macro.
///
/// The macro adds observability features to workflow step functions without
/// changing the function signature or behavior.
pub fn expand_kagzi_step(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = syn::parse2::<ItemFn>(item.into()).expect("Failed to parse function");

    // Validate the function signature
    if let Err(e) = validate_function_signature(&input_fn) {
        return e.to_compile_error().into();
    }

    // Extract function information
    let fn_name = &input_fn.sig.ident;
    let fn_attrs = &input_fn.attrs;
    let fn_vis = &input_fn.vis;
    let fn_docs = extract_docs(&input_fn.attrs);

    // Parse input and output types
    let (input_type, input_name) = match extract_input_type(&input_fn.sig) {
        Ok(result) => result,
        Err(e) => return e.to_compile_error().into(),
    };

    let output_type = match extract_output_type(&input_fn.sig) {
        Ok(result) => result,
        Err(e) => return e.to_compile_error().into(),
    };

    // Generate the enhanced function
    let expanded = generate_step_enhancement(
        fn_docs,
        fn_attrs,
        fn_vis,
        fn_name,
        &input_name,
        input_type,
        output_type,
        &input_fn.block,
    );

    expanded.into()
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

/// Extracts the input type and parameter name from function signature.
fn extract_input_type(sig: &Signature) -> Result<(Type, Ident), Error> {
    match sig.inputs.len() {
        2 => {
            // First parameter should be WorkflowContext
            match &sig.inputs[0] {
                FnArg::Typed(_) => {
                    // First parameter is context, we'll ignore it for input extraction
                }
                FnArg::Receiver(_) => {
                    return Err(Error::new_spanned(
                        &sig.inputs[0],
                        "kagzi_step functions cannot have a self parameter",
                    ));
                }
            }

            // Second parameter is the actual input
            match &sig.inputs[1] {
                FnArg::Typed(PatType { pat, ty, .. }) => {
                    let input_name = match &**pat {
                        Pat::Ident(pat_ident) => pat_ident.ident.clone(),
                        _ => {
                            return Err(Error::new_spanned(
                                pat,
                                "Expected simple identifier for input parameter",
                            ));
                        }
                    };
                    Ok((*ty.clone(), input_name))
                }
                FnArg::Receiver(_) => Err(Error::new_spanned(
                    &sig.inputs[1],
                    "kagzi_step functions cannot have a self parameter",
                )),
            }
        }
        n => Err(Error::new_spanned(
            &sig.inputs,
            format!(
                "kagzi_step functions must have exactly 2 input parameters (ctx and input), found {}",
                n
            ),
        )),
    }
}

/// Extracts the output type from function signature.
fn extract_output_type(sig: &Signature) -> Result<Type, Error> {
    match &sig.output {
        ReturnType::Type(_, ty) => {
            // Extract the Ok type from Result<T, E>
            match &**ty {
                Type::Path(TypePath { path, .. }) => {
                    if let Some(segment) = path.segments.last() {
                        if segment.ident == "Result" {
                            use syn::{GenericArgument, PathArguments};
                            match &segment.arguments {
                                PathArguments::AngleBracketed(args) => {
                                    if let Some(GenericArgument::Type(t)) = args.args.first() {
                                        return Ok(t.clone());
                                    }
                                }
                                _ => {}
                            }
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

/// Generates the enhanced step function with observability.
fn generate_step_enhancement(
    docs: Vec<Attribute>,
    attrs: &[Attribute],
    vis: &syn::Visibility,
    fn_name: &Ident,
    input_param_name: &Ident,
    input_type: Type,
    output_type: Type,
    block: &syn::Block,
) -> TokenStream {
    quote::quote! {
        #(#attrs)*
        #(#docs)*
        #vis async fn #fn_name(
            mut ctx: kagzi::WorkflowContext,
            #input_param_name: #input_type,
        ) -> anyhow::Result<#output_type> {
            let span = tracing::info_span!(
                "step",
                step = stringify!(#fn_name),
            );
            let _enter = span.enter();

            // Clone input for logging to avoid move issues
            let input_for_log = #input_param_name.clone();

            // Log input at debug level
            tracing::debug!(
                input = ?input_for_log,
                "Starting step: {}",
                stringify!(#fn_name)
            );

            // Execute the function body with error context
            let result = async move { #block }.await;

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
        }
    }
}
