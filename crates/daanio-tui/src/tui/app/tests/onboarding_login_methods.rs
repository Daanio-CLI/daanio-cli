// Daanio-specific onboarding login-method recovery tests.

#[test]
fn daanio_login_failure_returns_to_method_choice_for_retry() {
    with_temp_daanio_home(|| {
        let mut app = create_test_app();
        app.onboarding_flow = Some(OnboardingFlow {
            phase: OnboardingPhase::LoginOpenAi {
                yes_highlighted: false,
            },
        });
        app.onboarding_import_failed_provider = Some("daanio".to_string());
        app.onboarding_handle_login_failed(Some("Daanio rejected the key".to_string()));
        assert!(matches!(
            app.onboarding_phase(),
            Some(OnboardingPhase::Login { import: None })
        ));
        assert!(app.handle_onboarding_continue_prompt_key(KeyCode::Enter));
        assert!(matches!(
            app.onboarding_phase(),
            Some(OnboardingPhase::LoginOpenAi {
                yes_highlighted: true
            })
        ));
        assert!(app.onboarding_import_error.is_none());
    });
}
