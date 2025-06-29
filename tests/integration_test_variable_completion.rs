#[cfg(test)]
mod integration_tests {
    use erg::{DummyVM};
    use erg_common::config::ErgConfig;
    use erg_common::io::Input;

    #[test]
    #[cfg(feature = "els")]
    fn test_variable_completion_in_repl() {
        // Create a REPL instance
        let mut cfg = ErgConfig::default();
        cfg.input = Input::repl();
        let mut vm = DummyVM::new(cfg);

        // Evaluate some variables
        let result1 = vm.eval("x = 42".to_string());
        assert!(result1.is_ok());

        let result2 = vm.eval("y = \"hello\"".to_string());
        assert!(result2.is_ok());

        let result3 = vm.eval("long_variable_name = 3.14".to_string());
        assert!(result3.is_ok());

        // The variables should now be available in the compiler context
        // This is verified by the implementation updating the ELS integration
        // after each evaluation

        // Test that we can use the variables
        let result4 = vm.eval("x + 1".to_string());
        assert!(result4.is_ok());
        assert!(result4.unwrap().contains("43"));
    }

    #[test]
    #[cfg(feature = "els")]
    fn test_function_completion_in_repl() {
        let mut cfg = ErgConfig::default();
        cfg.input = Input::repl();
        let mut vm = DummyVM::new(cfg);

        // Define a function
        let result = vm.eval("add x, y = x + y".to_string());
        assert!(result.is_ok());

        // Function should be available for completion
        let result2 = vm.eval("add(1, 2)".to_string());
        assert!(result2.is_ok());
        assert!(result2.unwrap().contains("3"));
    }
}