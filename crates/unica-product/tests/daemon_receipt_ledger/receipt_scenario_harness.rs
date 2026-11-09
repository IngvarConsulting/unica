//! Проверки обвязки сценариев. Тела живут в библиотеке рядом с её закрытыми
//! типами (`tests/support/receipt_scenario/tests.rs`); здесь — только `#[test]`
//! на каждую из них, чтобы вариант с признаком не собирал тестовый `unica-daemon`.

unica_daemon::receipt_scenario_harness_tests!();
