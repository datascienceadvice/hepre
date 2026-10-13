//! Детерминированная элементарная математика.
//!
//! Воспроизводимость результатов исследования требует, чтобы генерация
//! синтетической когорты и эталонная логистическая модель не зависели от
//! реализации `libm` конкретной платформы: `f64::exp` и `f64::ln` **не**
//! специфицированы стандартом IEEE-754 и могут отличаться на последних битах
//! между платформами и версиями системных библиотек.
//!
//! Поэтому `exp` и `ln` здесь реализованы самостоятельно — только через
//! `+`, `-`, `*`, `/` и точное масштабирование степени двойки через
//! [`f64::to_bits`]/[`f64::from_bits`]. Такие операции специфицированы
//! IEEE-754 побитово, поэтому результат одинаков на любой платформе.
//!
//! Квадратный корень намеренно берётся из `std`: `sqrt` — единственная
//! трансцендентная по виду функция, которая по IEEE-754 обязана быть
//! корректно округлённой, а значит тоже платформо-независима.
//!
//! Точность реализаций проверяется в `tests/detmath.rs` сравнением со
//! `std` на сетке аргументов.

/// `sqrt` из `std`: корректно округлён по IEEE-754, поэтому детерминирован.
#[inline]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

/// `ln(2)` с точностью, достаточной для разложения (53 бита мантиссы).
const LN2: f64 = std::f64::consts::LN_2;

/// `1/sqrt(2)` — граница приведения мантиссы в [`ln`].
const SQRT_HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Точное умножение на степень двойки `2^k`.
///
/// Реализовано через битовую сборку экспоненты, а не через `powi`, поэтому
/// не зависит от реализации возведения в степень.
fn pow2i(k: i32) -> f64 {
    if k > 1023 {
        return f64::INFINITY;
    }
    if k < -1074 {
        return 0.0;
    }
    if k >= -1022 {
        // Нормализованное значение: экспонента = k + bias.
        f64::from_bits(((k + 1023) as u64) << 52)
    } else {
        // Субнормализованное значение.
        f64::from_bits(1u64 << (k + 1074))
    }
}

/// Детерминированный натуральный логарифм.
///
/// Приведение аргумента: `x = m · 2^k`, где `m ∈ [1/√2, √2)`. Затем
/// `ln m` через ряд артефакта `atanh`: `ln m = 2·(z + z³/3 + z⁵/5 + …)`,
/// `z = (m-1)/(m+1)`, `|z| ≤ 0.1716` — ряд сходится очень быстро.
///
/// Граничные случаи: `ln(0) = -∞`, `ln(x<0) = NaN`, `ln(∞) = ∞`, `ln(NaN) = NaN`.
pub fn ln(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }

    // Разбор представления: экспонента и мантисса.
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let raw_frac = bits & 0x000f_ffff_ffff_ffff;

    let (m, mut k) = if raw_exp == 0 {
        // Субнормализованное число: нормируем умножением на 2^54.
        let scaled = x * pow2i(54);
        let b = scaled.to_bits();
        let e = ((b >> 52) & 0x7ff) as i32 - 1023 - 54;
        (f64::from_bits((b & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000), e)
    } else {
        (f64::from_bits(raw_frac | 0x3ff0_0000_0000_0000), raw_exp - 1023)
    };

    // Приводим m к [1/√2, √2), чтобы |z| был мал.
    let mut m = m;
    if m > std::f64::consts::SQRT_2 {
        m *= 0.5;
        k += 1;
    }
    debug_assert!(m >= SQRT_HALF && m <= std::f64::consts::SQRT_2);

    let z = (m - 1.0) / (m + 1.0);
    let z2 = z * z;
    let mut term = z;
    let mut sum = 0.0_f64;
    // 18 членов: |z|^37 ≤ 0.1716^37 ≈ 1e-28 — далеко за пределами точности f64.
    for i in 0..18 {
        sum += term / (2 * i + 1) as f64;
        term *= z2;
    }
    let ln_m = 2.0 * sum;
    ln_m + (k as f64) * LN2
}

/// Детерминированная экспонента.
///
/// Приведение: `exp(x) = 2^k · exp(r)`, `k = round(x/ln2)`, `r = x - k·ln2`,
/// `|r| ≤ ln2/2 ≈ 0.347`. Далее ряд Тейлора (18 членов; остаток < 1e-20).
/// Умножение на `2^k` — точное ([`pow2i`]).
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x > 709.782712893384 {
        return f64::INFINITY;
    }
    if x < -745.1332191019411 {
        return 0.0;
    }

    let k = (x / LN2).round();
    let r = x - k * LN2;

    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    for i in 1..=18 {
        term *= r / i as f64;
        sum += term;
    }
    sum * pow2i(k as i32)
}

/// Логистическая функция `σ(x) = 1/(1+exp(-x))`.
///
/// Форма ветвления выбрана так, чтобы не переполняться ни на одном конце:
/// для `x ≥ 0` считается `1/(1+exp(-x))`, иначе `exp(x)/(1+exp(x))`.
pub fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        let e = exp(-x);
        1.0 / (1.0 + e)
    } else {
        let e = exp(x);
        e / (1.0 + e)
    }
}

/// Логит `ln(p/(1-p))`. Возвращает `±∞` на границах, `NaN` вне `[0,1]`.
pub fn logit(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    ln(p) - ln(1.0 - p)
}

/// Возведение в целую степень повторным умножением.
///
/// Не используем `f64::powi`: его реализация не гарантирует одинаковой
/// последовательности операций между сборками.
pub fn powi(base: f64, n: u32) -> f64 {
    let mut acc = 1.0_f64;
    let mut b = base;
    let mut e = n;
    while e > 0 {
        if e & 1 == 1 {
            acc *= b;
        }
        b *= b;
        e >>= 1;
    }
    acc
}

/// Детерминированный косинус.
///
/// Нужен для построения узлов Чебышёва при аппроксимации сигмоиды
/// многочленом: коэффициенты многочлена влияют на численный результат,
/// поэтому `f64::cos` здесь не используется.
///
/// Приведение: `x` по модулю `2π` → знак и отражение в `[0, π/2]`, затем
/// ряд Тейлора (24 члена; остаток при `x ≤ π/2` меньше 1e-17).
pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let two_pi = 2.0 * std::f64::consts::PI;
    let mut r = x - (x / two_pi).floor() * two_pi;
    let mut sign = 1.0_f64;

    if r > std::f64::consts::PI {
        r = two_pi - r;
    }
    if r > std::f64::consts::FRAC_PI_2 {
        r = std::f64::consts::PI - r;
        sign = -1.0;
    }

    // Ряд Тейлора для cos на [0, π/2].
    let r2 = r * r;
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    for k in 1..=24u32 {
        term *= -r2 / ((2 * k - 1) * (2 * k)) as f64;
        sum += term;
    }
    sign * sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_matches_std_on_grid() {
        let mut x = -40.0_f64;
        while x <= 40.0 {
            let got = exp(x);
            let want = x.exp();
            let rel = if want == 0.0 { got.abs() } else { ((got - want) / want).abs() };
            assert!(rel < 1e-14, "exp({x}): got {got}, want {want}, rel {rel:e}");
            x += 0.013;
        }
    }

    #[test]
    fn ln_matches_std_on_grid() {
        let mut x = 1e-6_f64;
        while x <= 1e6 {
            let got = ln(x);
            let want = x.ln();
            let rel = ((got - want) / want).abs();
            assert!(rel < 1e-13, "ln({x}): got {got}, want {want}, rel {rel:e}");
            x *= 1.017;
        }
    }

    #[test]
    fn sigmoid_is_symmetric_and_bounded() {
        for &x in &[0.0_f64, 0.5, 1.0, 5.0, 20.0, 100.0] {
            let a = sigmoid(x);
            let b = sigmoid(-x);
            assert!((a + b - 1.0).abs() < 1e-15, "σ({x}) + σ(-{x}) != 1");
            assert!((0.0..=1.0).contains(&a));
        }
        assert_eq!(sigmoid(0.0), 0.5);
    }

    #[test]
    fn logit_inverts_sigmoid() {
        for &x in &[-8.0_f64, -2.0, -0.5, 0.0, 0.5, 2.0, 8.0] {
            let p = sigmoid(x);
            let back = logit(p);
            assert!((back - x).abs() < 1e-11, "logit(σ({x})) = {back}");
        }
    }

    #[test]
    fn cos_matches_std_on_grid() {
        let mut x = -6.5_f64;
        while x <= 6.5 {
            let got = cos(x);
            let want = x.cos();
            assert!((got - want).abs() < 1e-15, "cos({x}): got {got}, want {want}");
            x += 0.007;
        }
        // Точки, важные для узлов Чебышёва.
        assert!((cos(0.0) - 1.0).abs() < 1e-16);
        assert!(cos(std::f64::consts::FRAC_PI_2).abs() < 1e-16);
        assert!((cos(std::f64::consts::PI) + 1.0).abs() < 1e-15);
    }

    #[test]
    fn boundary_values_are_sane() {
        assert!(ln(0.0).is_infinite() && ln(0.0) < 0.0);
        assert!(ln(-1.0).is_nan());
        assert!(ln(f64::INFINITY).is_infinite());
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(-1000.0), 0.0);
        assert!(exp(1000.0).is_infinite());
        assert_eq!(powi(2.0, 10), 1024.0);
        assert_eq!(powi(3.0, 0), 1.0);
    }
}
