//! How far a number from the solver may be from the one an infinitely fine mesh would give
//! (PIL-294 A6): Richardson extrapolation over meshes each finer than the last by one ratio, with
//! the order of convergence the meshes themselves show, and an error band from it.
//!
//! The method is the grid convergence study of verification practice (P. J. Roache, Verification
//! and Validation in Computational Science and Engineering, Hermosa 1998; I. B. Celik et al.,
//! "Procedure for Estimation and Reporting of Uncertainty Due to Discretization in CFD
//! Applications", J. Fluids Eng. 130, 2008): with values f1, f2, f3 on meshes of size h, h/r and
//! h/r², the changes e21 = f2 − f1 and e32 = f3 − f2 give the observed order p = ln(e21/e32) /
//! ln r, the extrapolated value f3 + e32 / (r^p − 1), and a band of 1.25 times the distance
//! between the two (Roache's safety factor for three meshes).

/// A number is settled when its band is at most this, %.
pub const SETTLED_BAND_PERCENT: f64 = 3.0;
/// The observed orders the extrapolation is trusted for. Ten-node tetrahedra converge a smooth
/// stress at about order 2; under 0.5 the number grows (or falls) without settling, as a peak at
/// a sharp inside corner or an idealised support does; over 4 the first mesh was too coarse for
/// the steps to say how fast the rest converges, and the two finer meshes decide alone.
pub const ORDER_RANGE: (f64, f64) = (0.5, 4.0);
/// The band is this many times the distance between the finest value and the extrapolated one
/// (Roache's factor of safety for a three-mesh study with an observed order).
pub const SAFETY_FACTOR: f64 = 1.25;
/// A step that moves a number by no more than this share of it (0.25 %) is the mesh's noise: two
/// such steps say the number has settled, and their ratio says nothing about an order. A peak at
/// a real singularity rises by several % a step at a ratio of 1.5 (V-7's sharp corner: 8 %), so
/// this hides none.
pub const NOISE: f64 = 0.0025;
/// The narrowest band, as a share of the value (±0.5 %). Meshes of the same size made another way
/// give a peak up to about that much apart, which one family of meshes cannot see: Peterson's
/// filleted bar (V-6) at 0.889 mm gave 109.97 MPa meshed whole and 109.75 MPa meshed as a quarter
/// on two symmetry planes, and the quarter's finer meshes settled at about 109.6 (PIL-294 A6,
/// agreed 2026-11-20).
pub const MIN_BAND: f64 = 0.005;

/// Which way a number is going with the mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trend {
    /// Within its band, which is at most [`SETTLED_BAND_PERCENT`]; with three meshes, at an order
    /// within [`ORDER_RANGE`], or over it with the two finer meshes within the band, or moving by
    /// no more than [`NOISE`] at each step.
    Settled,
    /// Converging, but its band is wider than [`SETTLED_BAND_PERCENT`], or the meshes give no
    /// order to trust (it goes up and down).
    Changing,
    /// It grows without settling (an order under 0.5): a sharp inside corner or an idealised
    /// support, where the stress has no finite value.
    Rising,
    /// The same, falling.
    Falling,
}

/// How the band was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// Two meshes: |f2 − f1| / (r − 1), the error left on the finer mesh when the error falls at
    /// least as fast as the element size (an order of 1 or more).
    TwoMeshes,
    /// Three meshes with an order in [`ORDER_RANGE`]: Richardson's extrapolation, the band
    /// [`SAFETY_FACTOR`] times its distance from the finest value.
    Extrapolated,
    /// Three meshes whose steps are both within [`NOISE`]: the two-mesh bound of the larger step.
    Noise,
    /// Three meshes whose order is over 4: the first mesh was too coarse to count, and the two
    /// finer ones give the two-mesh bound of their step.
    FastOrder,
    /// Three meshes that go up then down (or down then up): the larger step.
    Oscillating,
    /// Three meshes with an order under 0.5: no band.
    NoLimit,
}

/// What the meshes say about one number.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    /// The value to quote: the extrapolated one ([`Basis::Extrapolated`]), else the finest mesh's.
    pub value: f64,
    /// The finest mesh's value.
    pub finest: f64,
    /// How far `value` may be from the converged value, in the number's own units, at least
    /// [`MIN_BAND`] of it; `None` when it has none ([`Trend::Rising`], [`Trend::Falling`]).
    pub error: Option<f64>,
    /// The observed order, when three meshes gave one.
    pub order: Option<f64>,
    /// How many meshes it rests on: 2 or 3.
    pub meshes: usize,
    pub trend: Trend,
    pub basis: Basis,
}

impl Estimate {
    /// The band as a share of the value quoted (0.02 is ±2 %).
    pub fn band(&self) -> Option<f64> {
        let e = self.error?;
        Some(if self.value.abs() > 1e-12 {
            e / self.value.abs()
        } else if e > 1e-12 {
            f64::INFINITY
        } else {
            0.0
        })
    }
    /// Whether the value is Richardson's extrapolation rather than a mesh's own value.
    pub fn extrapolated(&self) -> bool {
        self.basis == Basis::Extrapolated
    }
}

/// What meshes of sizes h, h/`ratio`, h/`ratio`² gave for one number (`values`, first to finest:
/// two or three of them) says about it. `None` for fewer than two values, more than three, a value
/// that is not finite, or a ratio not above 1.
pub fn estimate(values: &[f64], ratio: f64) -> Option<Estimate> {
    if !(ratio.is_finite() && ratio > 1.0) || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    // A band of at least `MIN_BAND`, and whether it is within the settled band.
    let bounded = |value: f64, error: f64| {
        let error = error.max(MIN_BAND * value.abs());
        let within = error <= (SETTLED_BAND_PERCENT / 100.0) * value.abs() + 1e-12;
        (error, if within { Trend::Settled } else { Trend::Changing })
    };
    match *values {
        [f1, f2] => {
            let (error, trend) = bounded(f2, (f2 - f1).abs() / (ratio - 1.0));
            Some(Estimate { value: f2, finest: f2, error: Some(error), order: None, meshes: 2, trend, basis: Basis::TwoMeshes })
        }
        [f1, f2, f3] => {
            let (e21, e32) = (f2 - f1, f3 - f2);
            let scale = f3.abs().max(f2.abs()).max(1e-12);
            let three = |value: f64, error: Option<f64>, order: Option<f64>, trend: Trend, basis: Basis| Estimate {
                value,
                finest: f3,
                error,
                order,
                meshes: 3,
                trend,
                basis,
            };
            if e21.abs() <= NOISE * scale && e32.abs() <= NOISE * scale {
                let (error, trend) = bounded(f3, e21.abs().max(e32.abs()) / (ratio - 1.0));
                return Some(three(f3, Some(error), None, trend, Basis::Noise));
            }
            if e21 * e32 < 0.0 || e21 == 0.0 {
                // Up then down: the converged value lies within the swing (or the first step did
                // not move and the second did, which no order explains). No order, so never
                // settled.
                let (error, _) = bounded(f3, e21.abs().max(e32.abs()));
                return Some(three(f3, Some(error), None, Trend::Changing, Basis::Oscillating));
            }
            let order = (e21 / e32).ln() / ratio.ln();
            // Growing in size (a stress, a deflection) or shrinking.
            let up = e32.signum() == f3.signum() || f3 == 0.0;
            if order < ORDER_RANGE.0 {
                let trend = if up { Trend::Rising } else { Trend::Falling };
                return Some(three(f3, None, Some(order), trend, Basis::NoLimit));
            }
            if order > ORDER_RANGE.1 {
                let (error, trend) = bounded(f3, e32.abs() / (ratio - 1.0));
                return Some(three(f3, Some(error), Some(order), trend, Basis::FastOrder));
            }
            let extrapolated = f3 + e32 / (ratio.powf(order) - 1.0);
            let (error, trend) = bounded(extrapolated, SAFETY_FACTOR * (extrapolated - f3).abs());
            Some(three(extrapolated, Some(error), Some(order), trend, Basis::Extrapolated))
        }
        _ => None,
    }
}
