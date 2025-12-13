//! Procedural macros for the Kagzi workflow engine.
//!
//! This crate provides macros to reduce boilerplate when defining workflows and steps.
//! The macros enhance observability and provide a clean syntax for workflow definitions.

mod step_macro;
mod workflow_macro;

use proc_macro::TokenStream;

/// Enhances a workflow step function with observability features.
///
/// This attribute macro adds:
/// - Automatic tracing span creation with step name
/// - Input/output logging at debug level
/// - Error context enrichment
/// - Structured logging for better observability
///
/// # Examples
///
/// ```rust,ignore
/// // This is a simplified example - in actual usage, you would have
/// // proper types defined for OrderRequest, ValidationResult, etc.
/// use kagzi_macros::kagzi_step;
/// use kagzi::WorkflowContext;
///
/// #[kagzi_step]
/// /// Validates an order and calculates total
/// async fn validate_order(
///     ctx: WorkflowContext,
///     input: OrderRequest,
/// ) -> anyhow::Result<ValidationResult> {
///     if input.items.is_empty() {
///         return Err(OrderError::EmptyOrder.into());
///     }
///
///     let total: f64 = input.items.iter()
///         .map(|i| i.price * i.quantity as f64)
///         .sum();
///
///     Ok(ValidationResult {
///         is_valid: true,
///         total_amount: total,
///     })
/// }
/// ```
///
/// The macro expands to include tracing span creation, logging, and error context:
/// ```rust,ignore
/// async fn validate_order(
///     ctx: WorkflowContext,
///     input: OrderRequest,
/// ) -> anyhow::Result<ValidationResult> {
///     let span = tracing::info_span!("step", step = "validate_order");
///     let _enter = span.enter();
///
///     tracing::debug!(input = ?input, "Starting step: validate_order");
///
///     // Your function body with error context
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn kagzi_step(attrs: TokenStream, item: TokenStream) -> TokenStream {
    step_macro::expand_kagzi_step(attrs.into(), item.into()).into()
}

/// Creates a workflow with a clean, block-based syntax.
///
/// This macro provides a `run!` helper for executing steps and automatically
/// adds workflow-level tracing. It makes workflow definitions more readable
/// while maintaining full type safety.
///
/// # Examples
///
/// ```rust,ignore
/// // This is a simplified example - in actual usage, you would have
/// // proper types defined for OrderRequest, OrderResult, etc.
/// use kagzi_macros::kagzi_workflow;
/// use kagzi::WorkflowContext;
///
/// kagzi_workflow! {
///     pub async fn process_order(
///         mut ctx: WorkflowContext,
///         input: OrderRequest,
///     ) -> anyhow::Result<OrderResult> {
///         // Run validation step
///         let validated = run!("validate", validate_order(&input));
///
///         // Run inventory reservation
///         let inventory = run!("reserve", reserve_inventory(&validated));
///
///         // Run payment processing
///         let payment = run!("pay", process_payment(&validated));
///
///         // Ship the order
///         let shipment = run!("ship", ship_order(&input, &inventory, &payment));
///
///         // Return final result
///         Ok(OrderResult {
///             order_id: input.order_id,
///             inventory,
///             payment,
///             shipment,
///             status: "completed".to_string(),
///         })
///     }
/// }
/// ```
///
/// The `run!` macro simplifies step execution by:
/// - Automatically passing the context and input
/// - Using the provided string as the step name for tracing
/// - Propagating errors with the `?` operator
#[proc_macro]
pub fn kagzi_workflow(input: TokenStream) -> TokenStream {
    workflow_macro::expand_kagzi_workflow(input)
}

/// Helper macro for executing steps within a `kagzi_workflow!`.
///
/// This macro is automatically defined within workflow functions and
/// provides a simple way to execute workflow steps with proper tracing.
///
/// # Note
///
/// This macro is only available inside functions created with `kagzi_workflow!`.
#[doc(hidden)]
#[proc_macro]
pub fn run(_input: TokenStream) -> TokenStream {
    // This is a placeholder - the actual run! macro is generated
    // inside each workflow function by kagzi_workflow!
    TokenStream::new()
}

#[cfg(test)]
mod tests;
