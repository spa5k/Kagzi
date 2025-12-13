//! Tests for the Kagzi procedural macros.

#[cfg(test)]
mod macro_tests {

    // Test that the functions exist and can be called
    #[test]
    fn test_step_macro_function_exists() {
        // Just test that we can access the function
        let _: fn(proc_macro::TokenStream, proc_macro::TokenStream) -> proc_macro::TokenStream =
            crate::kagzi_step;
    }

    #[test]
    fn test_kagzi_workflow_exists() {
        // Just test that we can access the function
        let _: fn(proc_macro::TokenStream) -> proc_macro::TokenStream = crate::kagzi_workflow;
    }
}
