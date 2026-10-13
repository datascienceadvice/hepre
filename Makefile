# Makefile — тонкая обёртка над cargo/pwsh для HEPRE (Windows x64).
# Требуется GNU make и pwsh в PATH; PWSH можно переопределить (например, PWSH=powershell).
# Длинные сценарии живут в scripts/*.ps1 — здесь только делегирование, по одной команде на цель.

PWSH ?= pwsh
PYTHON ?= python
CARGO ?= cargo
CONFIG ?= configs/study.json
# Два прогона для проверки воспроизводимости (см. targets repro и compare).
RUN_A ?= results/20260925-042702_4659cac3
RUN_B ?= results/20261012-083347_4659cac3

.PHONY: build build-release test test-release bench study fmt clippy docs clean repro metadata compare

build:          ## Debug-сборка крэйта
	$(CARGO) build
build-release:  ## Release-сборка крэйта и бинарников ролей
	$(CARGO) build --release --bins
test:           ## Интеграционные тесты (debug)
	$(CARGO) test
test-release:   ## Все тесты (release): 71 модульный + 15 интеграционных
	$(CARGO) test --release
bench:          ## Бенчмарк сквозного скоринга (examples/bench.rs)
	$(CARGO) run --release --example bench
study:          ## Раннер исследования с конфигурацией $(CONFIG) -> results/
	$(CARGO) run --release --bin study -- --config $(CONFIG)
fmt:            ## Форматирование исходников
	$(CARGO) fmt --all
clippy:         ## Линты (предупреждения = ошибки)
	$(CARGO) clippy --all-targets -- -D warnings
docs:           ## Сборка документации крэйта
	$(CARGO) doc --no-deps
clean:          ## Очистка артефактов cargo
	$(CARGO) clean
repro:          ## Полное воспроизведение: setup-check -> build -> tests -> study
	$(PWSH) -NoProfile -File scripts/repro.ps1
metadata:       ## Структурная проверка CITATION.cff и ci.yml
	$(PYTHON) scripts/validate_metadata.py
compare:        ## Сверка двух прогонов на воспроизводимость
	$(PYTHON) scripts/compare_runs.py $(RUN_A) $(RUN_B)
