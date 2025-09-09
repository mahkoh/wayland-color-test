use {
    crate::{
        ordered_float::F64,
        protocols::color_management_v1::wp_color_manager_v1::{
            WpColorManagerV1Primaries, WpColorManagerV1TransferFunction,
        },
    },
    debug_fn::debug_fn,
    linearize::Linearize,
    std::{
        fmt::{Debug, Formatter},
        hash::{Hash, Hasher},
        marker::PhantomData,
        ops::{Mul, MulAssign},
    },
};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Luminance {
    pub min: F64,
    pub max: F64,
    pub white: F64,
}

pub struct ColorMatrix<To = Local, From = Local>(pub [[F64; 4]; 3], PhantomData<(To, From)>);

#[derive(Copy, Clone)]
pub struct Local;
#[derive(Copy, Clone)]
pub struct Xyz;
#[derive(Copy, Clone)]
pub struct Lms;
#[derive(Copy, Clone)]
pub struct Bradford;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Linearize)]
pub enum NamedTransferFunction {
    Srgb,
    Linear,
    St2084Pq,
    Bt1886,
    Gamma22,
    Gamma28,
    St240,
    ExtSrgb,
    Log100,
    Log316,
    St428,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Linearize)]
pub enum TransferFunction {
    Named(NamedTransferFunction),
    Pow,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TransferFunctionWithArgs {
    pub tf: TransferFunction,
    pub pow: f32,
}

impl NamedTransferFunction {
    pub const fn wayland(self) -> WpColorManagerV1TransferFunction {
        match self {
            NamedTransferFunction::Srgb => WpColorManagerV1TransferFunction::SRGB,
            NamedTransferFunction::Linear => WpColorManagerV1TransferFunction::EXT_LINEAR,
            NamedTransferFunction::St2084Pq => WpColorManagerV1TransferFunction::ST2084_PQ,
            NamedTransferFunction::Bt1886 => WpColorManagerV1TransferFunction::BT1886,
            NamedTransferFunction::Gamma22 => WpColorManagerV1TransferFunction::GAMMA22,
            NamedTransferFunction::Gamma28 => WpColorManagerV1TransferFunction::GAMMA28,
            NamedTransferFunction::St240 => WpColorManagerV1TransferFunction::ST240,
            NamedTransferFunction::ExtSrgb => WpColorManagerV1TransferFunction::EXT_SRGB,
            NamedTransferFunction::Log100 => WpColorManagerV1TransferFunction::LOG_100,
            NamedTransferFunction::Log316 => WpColorManagerV1TransferFunction::LOG_316,
            NamedTransferFunction::St428 => WpColorManagerV1TransferFunction::ST428,
        }
    }
}

pub fn matrix_from_lms(primaries: Primaries, luminance: Luminance) -> ColorMatrix<Local, Lms> {
    let (_, mut mat) = primaries.matrices();
    if luminance != Luminance::SRGB {
        mat *= white_balance(&Luminance::SRGB, &luminance, primaries.wp);
    }
    if primaries.wp != Primaries::SRGB.wp {
        mat *= bradford_adjustment(Primaries::SRGB.wp, primaries.wp);
    }
    mat * ColorMatrix::XYZ_FROM_LMS
}

impl Luminance {
    pub const SRGB: Self = Self {
        min: F64(0.2),
        max: F64(80.0),
        white: F64(80.0),
    };

    pub const BT1886: Self = Self {
        min: F64(0.01),
        max: F64(100.0),
        white: F64(100.0),
    };

    pub const ST2084_PQ: Self = Self {
        min: F64(0.0),
        max: F64(10000.0),
        white: F64(203.0),
    };

    #[expect(dead_code)]
    pub const HLG: Self = Self {
        min: F64(0.005),
        max: F64(1000.0),
        white: F64(203.0),
    };

    pub const WINDOWS_SCRGB: Self = Self {
        min: Self::ST2084_PQ.min,
        max: Self::ST2084_PQ.max,
        // This causes the white balance formula (with target ST2084_PQ) to simplify to
        // `Y * 80 / 10000`, meaning that sRGB pure white maps to a luminance of
        // 80 cd/m^2.
        white: F64(Self::ST2084_PQ.white.0 / 80.0 * Self::ST2084_PQ.max.0),
    };
}

impl Default for Luminance {
    fn default() -> Self {
        Self::SRGB
    }
}

#[expect(non_snake_case)]
pub fn white_balance(from: &Luminance, to: &Luminance, w_to: (F64, F64)) -> ColorMatrix<Xyz, Xyz> {
    let a = ((from.max - from.min) / (to.max - to.min) * (to.white - from.min)
        / (from.white - from.min))
        .0;
    let d = ((from.min - to.min) / (to.max - to.min)).0.max(0.0);
    let s = a - d;
    let (F64(x_to), F64(y_to)) = w_to;
    let X_to = x_to / y_to;
    let Y_to = 1.0;
    let Z_to = (1.0 - x_to - y_to) / y_to;
    ColorMatrix::new([
        [s, 0.0, 0.0, d * X_to],
        [0.0, s, 0.0, d * Y_to],
        [0.0, 0.0, s, d * Z_to],
    ])
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Linearize)]
pub enum NamedPrimaries {
    Srgb,
    PalM,
    Pal,
    Ntsc,
    GenericFilm,
    Bt2020,
    Cie1931Xyz,
    DciP3,
    DisplayP3,
    AdobeRgb,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Primaries {
    pub r: (F64, F64),
    pub g: (F64, F64),
    pub b: (F64, F64),
    pub wp: (F64, F64),
}

impl Primaries {
    pub const SRGB: Self = Self {
        r: (F64(0.64), F64(0.33)),
        g: (F64(0.3), F64(0.6)),
        b: (F64(0.15), F64(0.06)),
        wp: (F64(0.3127), F64(0.3290)),
    };

    pub const PAL_M: Self = Self {
        r: (F64(0.67), F64(0.33)),
        g: (F64(0.21), F64(0.71)),
        b: (F64(0.14), F64(0.08)),
        wp: (F64(0.310), F64(0.316)),
    };

    pub const PAL: Self = Self {
        r: (F64(0.64), F64(0.33)),
        g: (F64(0.29), F64(0.60)),
        b: (F64(0.15), F64(0.06)),
        wp: (F64(0.3127), F64(0.3290)),
    };

    pub const NTSC: Self = Self {
        r: (F64(0.630), F64(0.340)),
        g: (F64(0.310), F64(0.595)),
        b: (F64(0.155), F64(0.070)),
        wp: (F64(0.3127), F64(0.3290)),
    };

    pub const GENERIC_FILM: Self = Self {
        r: (F64(0.681), F64(0.319)),
        g: (F64(0.243), F64(0.692)),
        b: (F64(0.145), F64(0.049)),
        wp: (F64(0.310), F64(0.316)),
    };

    pub const BT2020: Self = Self {
        r: (F64(0.708), F64(0.292)),
        g: (F64(0.170), F64(0.797)),
        b: (F64(0.131), F64(0.046)),
        wp: (F64(0.3127), F64(0.3290)),
    };

    pub const CIE1931_XYZ: Self = Self {
        r: (F64(1.0), F64(0.0)),
        g: (F64(0.0), F64(1.0)),
        b: (F64(0.0), F64(0.0)),
        wp: (F64(1.0 / 3.0), F64(1.0 / 3.0)),
    };

    pub const DCI_P3: Self = Self {
        r: (F64(0.680), F64(0.320)),
        g: (F64(0.265), F64(0.690)),
        b: (F64(0.150), F64(0.060)),
        wp: (F64(0.314), F64(0.351)),
    };

    pub const DISPLAY_P3: Self = Self {
        r: (F64(0.680), F64(0.320)),
        g: (F64(0.265), F64(0.690)),
        b: (F64(0.150), F64(0.060)),
        wp: (F64(0.3127), F64(0.3290)),
    };

    pub const ADOBE_RGB: Self = Self {
        r: (F64(0.64), F64(0.33)),
        g: (F64(0.21), F64(0.71)),
        b: (F64(0.15), F64(0.06)),
        wp: (F64(0.3127), F64(0.3290)),
    };
}
impl NamedPrimaries {
    pub const fn primaries(self) -> Primaries {
        match self {
            NamedPrimaries::Srgb => Primaries::SRGB,
            NamedPrimaries::PalM => Primaries::PAL_M,
            NamedPrimaries::Pal => Primaries::PAL,
            NamedPrimaries::Ntsc => Primaries::NTSC,
            NamedPrimaries::GenericFilm => Primaries::GENERIC_FILM,
            NamedPrimaries::Bt2020 => Primaries::BT2020,
            NamedPrimaries::Cie1931Xyz => Primaries::CIE1931_XYZ,
            NamedPrimaries::DciP3 => Primaries::DCI_P3,
            NamedPrimaries::DisplayP3 => Primaries::DISPLAY_P3,
            NamedPrimaries::AdobeRgb => Primaries::ADOBE_RGB,
        }
    }

    pub const fn wayland(self) -> WpColorManagerV1Primaries {
        match self {
            NamedPrimaries::Srgb => WpColorManagerV1Primaries::SRGB,
            NamedPrimaries::PalM => WpColorManagerV1Primaries::PAL_M,
            NamedPrimaries::Pal => WpColorManagerV1Primaries::PAL,
            NamedPrimaries::Ntsc => WpColorManagerV1Primaries::NTSC,
            NamedPrimaries::GenericFilm => WpColorManagerV1Primaries::GENERIC_FILM,
            NamedPrimaries::Bt2020 => WpColorManagerV1Primaries::BT2020,
            NamedPrimaries::Cie1931Xyz => WpColorManagerV1Primaries::CIE1931_XYZ,
            NamedPrimaries::DciP3 => WpColorManagerV1Primaries::DCI_P3,
            NamedPrimaries::DisplayP3 => WpColorManagerV1Primaries::DISPLAY_P3,
            NamedPrimaries::AdobeRgb => WpColorManagerV1Primaries::ADOBE_RGB,
        }
    }
}

impl<T, U> Copy for ColorMatrix<T, U> {}

impl<T, U> Clone for ColorMatrix<T, U> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, U> PartialEq<Self> for ColorMatrix<T, U> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T, U> Eq for ColorMatrix<T, U> {}

impl<T, U> Hash for ColorMatrix<T, U> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T, U> Debug for ColorMatrix<T, U> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ColorMatrix")
            .field(&format_matrix(&self.0))
            .finish()
    }
}

fn format_matrix(m: &[[F64; 4]; 3]) -> impl Debug + use<'_> {
    debug_fn(move |f| {
        let iter = m
            .iter()
            .copied()
            .chain(Some([F64(0.0), F64(0.0), F64(0.0), F64(1.0)]))
            .enumerate();
        if f.alternate() {
            for (idx, row) in iter {
                if idx > 0 {
                    f.write_str("\n")?;
                }
                write!(
                    f,
                    "{:7.4} {:7.4} {:7.4} {:7.4}",
                    row[0], row[1], row[2], row[3]
                )?;
            }
        } else {
            f.write_str("[")?;
            for (idx, row) in iter {
                if idx > 0 {
                    f.write_str(", ")?;
                }
                write!(
                    f,
                    "[{:.4}, {:.4}, {:.4}, {:.4}]",
                    row[0], row[1], row[2], row[3]
                )?;
            }
            f.write_str("]")?;
        }
        Ok(())
    })
}

impl<T, U, V> Mul<ColorMatrix<U, T>> for ColorMatrix<V, U> {
    type Output = ColorMatrix<V, T>;

    fn mul(self, rhs: ColorMatrix<U, T>) -> Self::Output {
        let a = &self.0;
        let b = &rhs.0;
        macro_rules! mul {
            ($ar:expr, $bc:expr) => {
                a[$ar][0] * b[0][$bc] + a[$ar][1] * b[1][$bc] + a[$ar][2] * b[2][$bc]
            };
        }
        let m = [
            [mul!(0, 0), mul!(0, 1), mul!(0, 2), mul!(0, 3) + a[0][3]],
            [mul!(1, 0), mul!(1, 1), mul!(1, 2), mul!(1, 3) + a[1][3]],
            [mul!(2, 0), mul!(2, 1), mul!(2, 2), mul!(2, 3) + a[2][3]],
        ];
        ColorMatrix(m, PhantomData)
    }
}

impl<U, V> MulAssign<ColorMatrix<U, U>> for ColorMatrix<V, U> {
    fn mul_assign(&mut self, rhs: ColorMatrix<U, U>) {
        *self = *self * rhs;
    }
}

impl<T, U> Mul<[f64; 3]> for ColorMatrix<T, U> {
    type Output = [f64; 3];

    fn mul(self, rhs: [f64; 3]) -> Self::Output {
        let a = &self.0;
        macro_rules! mul {
            ($ar:expr) => {
                a[$ar][0].0 * rhs[0] + a[$ar][1].0 * rhs[1] + a[$ar][2].0 * rhs[2]
            };
        }
        [mul!(0), mul!(1), mul!(2)]
    }
}

impl<T, U> ColorMatrix<T, U> {
    pub const fn new(m: [[f64; 4]; 3]) -> Self {
        let m = [
            [F64(m[0][0]), F64(m[0][1]), F64(m[0][2]), F64(m[0][3])],
            [F64(m[1][0]), F64(m[1][1]), F64(m[1][2]), F64(m[1][3])],
            [F64(m[2][0]), F64(m[2][1]), F64(m[2][2]), F64(m[2][3])],
        ];
        Self(m, PhantomData)
    }

    pub const fn to_f32(self) -> [[f32; 4]; 4] {
        let m = self.0;
        macro_rules! map {
            ($r:expr, $c:expr) => {
                m[$r][$c].0 as f32
            };
        }
        [
            [map!(0, 0), map!(0, 1), map!(0, 2), map!(0, 3)],
            [map!(1, 0), map!(1, 1), map!(1, 2), map!(1, 3)],
            [map!(2, 0), map!(2, 1), map!(2, 2), map!(2, 3)],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}

impl ColorMatrix<Xyz, Lms> {
    const XYZ_FROM_LMS: Self = Self::new([
        [1.22701, -0.5578, 0.281256, 0.0],
        [-0.0405802, 1.11226, -0.0716767, 0.0],
        [-0.0763813, -0.421482, 1.58616, 0.0],
    ]);
}

impl ColorMatrix<Bradford, Xyz> {
    const BFD: Self = Self::new([
        [0.8951, 0.2664, -0.1614, 0.0],
        [-0.7502, 1.7135, 0.0367, 0.0],
        [0.0389, -0.0685, 1.0296, 0.0],
    ]);
}

impl ColorMatrix<Xyz, Bradford> {
    const BFD_INV: Self = Self::new([
        [0.9870, -0.1471, 0.1600, 0.0],
        [0.4323, 0.5184, 0.0493, 0.0],
        [-0.0085, 0.04, 0.9685, 0.0],
    ]);
}

#[expect(non_snake_case)]
pub fn bradford_adjustment(w_from: (F64, F64), w_to: (F64, F64)) -> ColorMatrix<Xyz, Xyz> {
    let (F64(x_from), F64(y_from)) = w_from;
    let (F64(x_to), F64(y_to)) = w_to;
    let X_from = x_from / y_from;
    let Z_from = (1.0 - x_from - y_from) / y_from;
    let X_to = x_to / y_to;
    let Z_to = (1.0 - x_to - y_to) / y_to;
    let [R_from, G_from, B_from] = ColorMatrix::BFD * [X_from, 1.0, Z_from];
    let [R_to, G_to, B_to] = ColorMatrix::BFD * [X_to, 1.0, Z_to];
    let adj = ColorMatrix::new([
        [R_to / R_from, 0.0, 0.0, 0.0],
        [0.0, G_to / G_from, 0.0, 0.0],
        [0.0, 0.0, B_to / B_from, 0.0],
    ]);
    ColorMatrix::BFD_INV * adj * ColorMatrix::BFD
}

impl Primaries {
    #[expect(non_snake_case)]
    pub const fn matrices(&self) -> (ColorMatrix<Xyz, Local>, ColorMatrix<Local, Xyz>) {
        let (F64(xw), F64(yw)) = self.wp;
        let Xw = xw / yw;
        let Zw = (1.0 - xw - yw) / yw;
        let (F64(xr), F64(yr)) = self.r;
        let (F64(xg), F64(yg)) = self.g;
        let (F64(xb), F64(yb)) = self.b;
        let zr = 1.0 - xr - yr;
        let zg = 1.0 - xg - yg;
        let zb = 1.0 - xb - yb;
        let srx = yg * zb - zg * yb;
        let sry = zg * xb - xg * zb;
        let srz = xg * yb - yg * xb;
        let sgx = zr * yb - yr * zb;
        let sgz = yr * xb - xr * yb;
        let sgy = xr * zb - zr * xb;
        let sbx = yr * zg - zr * yg;
        let sby = zr * xg - xr * zg;
        let sbz = xr * yg - yr * xg;
        let det = srz + sgz + sbz;
        let sr = srx * Xw + sry + srz * Zw;
        let sg = sgx * Xw + sgy + sgz * Zw;
        let sb = sbx * Xw + sby + sbz * Zw;
        let det_inv = 1.0 / det;
        let sr_inv = 1.0 / sr;
        let sg_inv = 1.0 / sg;
        let sb_inv = 1.0 / sb;
        let srp = sr * det_inv;
        let sgp = sg * det_inv;
        let sbp = sb * det_inv;
        let XYZ_from_local = [
            [srp * xr, sgp * xg, sbp * xb, 0.0],
            [srp * yr, sgp * yg, sbp * yb, 0.0],
            [srp * zr, sgp * zg, sbp * zb, 0.0],
        ];
        let local_from_XYZ = [
            [srx * sr_inv, sry * sr_inv, srz * sr_inv, 0.0],
            [sgx * sg_inv, sgy * sg_inv, sgz * sg_inv, 0.0],
            [sbx * sb_inv, sby * sb_inv, sbz * sb_inv, 0.0],
        ];
        (
            ColorMatrix::new(XYZ_from_local),
            ColorMatrix::new(local_from_XYZ),
        )
    }
}
