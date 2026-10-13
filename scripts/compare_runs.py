#!/usr/bin/env python3
"""Сравнение двух прогонов исследования — проверка воспроизводимости.

Запуск (из корня репозитория):

    python scripts/compare_runs.py results/<run-id-A> results/<run-id-B>

Величины делятся на три класса:

* **EXACT** — обязаны совпасть бит-в-бит. Сюда попадает всё, что не зависит
  ни от шума шифрования, ни от загрузки машины: когорта (по хэшу SHA-256),
  коэффициенты модели, коэффициенты многочлена, клинические метрики, число
  расхождений решений, объёмы ключей и шифротекстов.
* **ABS** — ограничены по абсолютной величине. Это величины, которые
  целиком состоят из шума CKKS (режим A: прямая передача логита), поэтому
  их точное значение невоспроизводимо по построению; содержательное
  утверждение — что они остаются пренебрежимо малы.
* **REL** — ограничены по относительной величине. Это режим B, где ошибку
  определяет детерминированная погрешность аппроксимации, а шум CKKS лишь
  шевелит последние значащие цифры.

Тайминги сравниваются информационно: они машинозависимы.

Код возврата ненулевой, если нарушен хотя бы один инвариант.

Вывод намеренно на английском: русский текст в консоли Windows с кодовой
страницей cp866 превращается в мусор.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

# --- инварианты -------------------------------------------------------------

#: Обязаны совпасть точно.
EXACT: list[str] = [
    "cohort.intercept_true",
    "cohort.target_prevalence",
    "cohort.realized_prevalence_derivation",
    "cohort.realized_prevalence_validation",
    "cohort.realized_prevalence_all",
    "cohort.eta_mean_all",
    "cohort.eta_sd_all",
    "cohort.eta_min_all",
    "cohort.eta_max_all",
    "cohort.feature_summary",
    "model.n_train",
    "model.n_events_train",
    "model.iterations",
    "model.converged",
    "model.lambda",
    "model.intercept_standardized",
    "model.weights_standardized",
    "model.means",
    "model.sds",
    "model.odds_ratios_per_unit",
    "model.events_per_variable",
    "model.feature_names",
    "polynomial.bound",
    "polynomial.max_abs_eta_observed",
    "polynomial.selected_power",
    "polynomial.selected_mult_depth",
    "polynomial.required_context_depth",
    "polynomial.context_depth_used",
    "polynomial.ring_dimension",
    "polynomial.slots_per_batch",
    "polynomial.max_abs_error",
    "polynomial.max_abs_error_on_data",
    "polynomial.sign_preserving",
    "polynomial.min_delta_positive_side",
    "polynomial.all_coefficients_nonnegative",
    "polynomial.monotone",
    "polynomial.decision_mismatches_on_data",
    "polynomial.decision_mismatches_on_grid",
    "polynomial.coeffs",
    "polynomial.sweep",
    "clinical_performance.auc",
    "clinical_performance.brier",
    "clinical_performance.calibration_intercept",
    "clinical_performance.calibration_slope",
    "clinical_performance.prevalence_validation",
    "clinical_performance.youden_threshold",
    "clinical_performance.at_default_threshold",
    "clinical_performance.at_youden_threshold",
    "clinical_performance.calibration_deciles",
    "he_probability_mode.discordant",
    "he_probability_mode.total",
    "he_probability_mode.discordance_rate",
    "he_probability_mode.discordance_ci",
    "he_probability_mode.sign_mismatches",
    "he_probability_mode.equivalence",
    "he_logit_mode.discordant",
    "he_logit_mode.total",
    "he_logit_mode.discordance_ci",
    "he_logit_mode.sign_mismatches",
    "he_logit_mode.equivalence",
    "determinism.cohort_sha256_first",
    "determinism.cohort_sha256_second",
    "determinism.cohort_bit_identical",
    "determinism.model_bit_identical",
    "determinism.polynomial_bit_identical",
    "keys_and_volume.public_key_bytes",
    "keys_and_volume.secret_key_bytes",
    "keys_and_volume.proxy_rekey_bytes",
    "keys_and_volume.feature_batch_bytes",
    "keys_and_volume.result_bytes",
    "keys_and_volume.n_patients",
    "sample_size_justification.upper_bound_with_zero_discordances",
    "sample_size_justification.max_discordances_within_margin",
]

#: (путь, абсолютный предел) — величины, состоящие из шума CKKS.
ABS: list[tuple[str, float]] = [
    ("he_logit_mode.max_abs_error", 1e-6),
    ("he_logit_mode.rmse", 1e-6),
    ("he_logit_mode.mean_abs_error", 1e-6),
]

#: (путь, относительный предел) — детерминированная ошибка плюс шум в младших цифрах.
REL: list[tuple[str, float]] = [
    ("he_probability_mode.max_abs_error", 1e-3),
    ("he_probability_mode.rmse", 1e-2),
    ("he_probability_mode.mean_abs_error", 1e-2),
]


def resolve(doc: dict, dotted: str):
    current = doc
    for part in dotted.split("."):
        current = current[part]
    return current


def load(run_dir: Path) -> dict:
    path = run_dir / "results.json"
    if not path.is_file():
        raise SystemExit(f"missing {path}")
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__)
        return 2

    dir_a, dir_b = Path(argv[1]), Path(argv[2])
    a, b = load(dir_a), load(dir_b)

    print(f"A = {dir_a}")
    print(f"B = {dir_b}")
    print()

    failures = 0

    # --- точное совпадение ---------------------------------------------------
    for path in EXACT:
        va, vb = resolve(a, path), resolve(b, path)
        if va != vb:
            failures += 1
            print(f"  MISMATCH (exact) {path}")
            print(f"      A: {va!r}")
            print(f"      B: {vb!r}")
    print(f"exact invariants: {len(EXACT) - failures}/{len(EXACT)} matched")

    # --- абсолютный предел ---------------------------------------------------
    abs_bad = 0
    for path, limit in ABS:
        va, vb = resolve(a, path), resolve(b, path)
        if max(abs(va), abs(vb)) > limit or va == 0.0 or vb == 0.0:
            abs_bad += 1
            failures += 1
            print(f"  MISMATCH (abs) {path}: A={va:.4e} B={vb:.4e} limit={limit:.0e}")
    print(f"absolute-bounded (pure CKKS noise, both must stay negligible): "
          f"{len(ABS) - abs_bad}/{len(ABS)} within limit")

    # --- относительный предел ------------------------------------------------
    rel_bad = 0
    for path, rtol in REL:
        va, vb = resolve(a, path), resolve(b, path)
        rel = abs(va - vb) / max(abs(va), abs(vb), 1e-300)
        if rel > rtol:
            rel_bad += 1
            failures += 1
            print(f"  MISMATCH (rel) {path}: A={va:.6e} B={vb:.6e} rel={rel:.2e} > {rtol:.0e}")
    print(f"relative-bounded (approximation error dominates): "
          f"{len(REL) - rel_bad}/{len(REL)} within tolerance")

    # --- когорта по хэшу -----------------------------------------------------
    sha_a = resolve(a, "determinism.cohort_sha256_first")
    sha_b = resolve(b, "determinism.cohort_sha256_first")
    print()
    print(f"cohort SHA-256 A = {sha_a}")
    print(f"cohort SHA-256 B = {sha_b}")
    print(f"cohort bit-identical across runs: {sha_a == sha_b}")

    # --- информационно: тайминги --------------------------------------------
    print()
    print("scalability (batch count and sign mismatches are invariants; times are not):")
    for pa, pb in zip(a["scalability"], b["scalability"]):
        print(
            f"  N={pa['requested_n']:>6}: batches {pa['batches']}->{pb['batches']}, "
            f"sign mismatches {pa['sign_mismatches']}->{pb['sign_mismatches']}, "
            f"total {pa['total_ms']:.0f} ms -> {pb['total_ms']:.0f} ms"
        )
        if pa["batches"] != pb["batches"] or pa["sign_mismatches"] != pb["sign_mismatches"]:
            failures += 1
            print("      ^ INVARIANT VIOLATION")

    print()
    if failures:
        print(f"REPRODUCIBILITY CHECK: FAILED ({failures} violation(s))")
        return 1
    print("REPRODUCIBILITY CHECK: PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
