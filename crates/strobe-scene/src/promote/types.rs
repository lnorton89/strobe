// ============================================================================
// Module:       strobe_scene::promote::types
// Description:  The GLSL types the int→float pass tracks, the built-in type
//               names, and the built-in functions' conversion rules.
//
// Dependencies: none
// ============================================================================

//! Just enough of GLSL's type system to know when an `int` meets a `float`.
//!
//! Numbers are a scalar kind and a shape (`rows` × `cols`: a scalar is 1×1,
//! a vector is n×1, a matrix is rows×cols). Anything the pass does not
//! follow is [`Ty::Unknown`], and the pass never edits against an unknown
//! type: a missed conversion leaves the scene as it was, a wrong one would
//! break a scene that compiled.

/// A scalar kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scalar {
    /// `float`, `vecN`, `matN`.
    Float,
    /// `int`, `ivecN`.
    Int,
    /// `uint`, `uvecN`.
    Uint,
    /// `bool`, `bvecN`.
    Bool,
}

/// A type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Ty {
    /// A scalar, vector or matrix.
    Num {
        /// Its scalar kind.
        scalar: Scalar,
        /// Components of a vector, rows of a matrix; 1 for a scalar.
        rows: u8,
        /// Columns of a matrix; 1 otherwise.
        cols: u8,
    },
    /// A sampler, with the number of components `textureSize` returns.
    Sampler(u8),
    /// A struct, by name.
    Struct(String),
    /// An array of the inner type.
    Array(Box<Ty>),
    /// `void`.
    Void,
    /// Anything not followed.
    Unknown,
}

impl Ty {
    /// A scalar of `scalar`.
    pub(super) const fn scalar(scalar: Scalar) -> Self {
        Self::Num {
            scalar,
            rows: 1,
            cols: 1,
        }
    }

    /// A vector (or scalar, for `rows == 1`) of `scalar`.
    pub(super) const fn vector(scalar: Scalar, rows: u8) -> Self {
        Self::Num {
            scalar,
            rows,
            cols: 1,
        }
    }

    /// `float`.
    pub(super) const FLOAT: Self = Self::scalar(Scalar::Float);
    /// `int`.
    pub(super) const INT: Self = Self::scalar(Scalar::Int);
    /// `bool`.
    pub(super) const BOOL: Self = Self::scalar(Scalar::Bool);

    /// The scalar kind of a number.
    pub(super) const fn kind(&self) -> Option<Scalar> {
        match self {
            Self::Num { scalar, .. } => Some(*scalar),
            Self::Sampler(_) | Self::Struct(_) | Self::Array(_) | Self::Void | Self::Unknown => {
                None
            }
        }
    }

    /// `true` for `float`, `vecN` and `matN`.
    pub(super) const fn is_float(&self) -> bool {
        matches!(self.kind(), Some(Scalar::Float))
    }

    /// `true` for `int`, `uint` and their vectors: what desktop GLSL
    /// converts to `float` implicitly.
    pub(super) const fn is_integer(&self) -> bool {
        matches!(self.kind(), Some(Scalar::Int | Scalar::Uint))
    }

    /// `true` for a one-component number.
    pub(super) const fn is_scalar(&self) -> bool {
        matches!(
            self,
            Self::Num {
                rows: 1,
                cols: 1,
                ..
            }
        )
    }

    /// The same shape with a float scalar kind; `None` for a non-number.
    pub(super) const fn as_float(&self) -> Option<Self> {
        match self {
            Self::Num { rows, cols, .. } => Some(Self::Num {
                scalar: Scalar::Float,
                rows: *rows,
                cols: *cols,
            }),
            Self::Sampler(_) | Self::Struct(_) | Self::Array(_) | Self::Void | Self::Unknown => {
                None
            }
        }
    }

    /// The same shape with a different scalar kind.
    pub(super) const fn with_kind(&self, scalar: Scalar) -> Self {
        match self {
            Self::Num { rows, cols, .. } => Self::Num {
                scalar,
                rows: *rows,
                cols: *cols,
            },
            Self::Sampler(_) | Self::Struct(_) | Self::Array(_) | Self::Void | Self::Unknown => {
                Self::Unknown
            }
        }
    }

    /// `(rows, cols)` of a number.
    pub(super) const fn shape(&self) -> Option<(u8, u8)> {
        match self {
            Self::Num { rows, cols, .. } => Some((*rows, *cols)),
            Self::Sampler(_) | Self::Struct(_) | Self::Array(_) | Self::Void | Self::Unknown => {
                None
            }
        }
    }

    /// The constructor that turns an integer value of this shape into
    /// floats: `float`, `vecN`. `None` for shapes with no integer form.
    pub(super) const fn float_constructor(&self) -> Option<&'static str> {
        match self.shape() {
            Some((1, 1)) => Some("float"),
            Some((2, 1)) => Some("vec2"),
            Some((3, 1)) => Some("vec3"),
            Some((4, 1)) => Some("vec4"),
            Some(_) | None => None,
        }
    }
}

/// The type a built-in type name stands for.
pub(super) fn builtin_type(name: &str) -> Option<Ty> {
    let vector = |prefix: &str, scalar: Scalar| -> Option<Ty> {
        let rows = name.strip_prefix(prefix)?.parse::<u8>().ok()?;
        (2..=4).contains(&rows).then(|| Ty::vector(scalar, rows))
    };
    let ty = match name {
        "float" => Ty::FLOAT,
        "int" => Ty::INT,
        "uint" => Ty::scalar(Scalar::Uint),
        "bool" => Ty::BOOL,
        "void" => Ty::Void,
        "sampler1D" | "isampler1D" | "usampler1D" => Ty::Sampler(1),
        "sampler3D" | "isampler3D" | "usampler3D" | "sampler2DArray" | "isampler2DArray"
        | "usampler2DArray" => Ty::Sampler(3),
        "sampler2D" | "isampler2D" | "usampler2D" | "samplerCube" | "isamplerCube"
        | "usamplerCube" | "sampler2DShadow" | "samplerCubeShadow" | "sampler2DRect" => {
            Ty::Sampler(2)
        }
        _ if name.starts_with("mat") => return matrix(name),
        _ if name.starts_with("vec") => return vector("vec", Scalar::Float),
        _ if name.starts_with("ivec") => return vector("ivec", Scalar::Int),
        _ if name.starts_with("uvec") => return vector("uvec", Scalar::Uint),
        _ if name.starts_with("bvec") => return vector("bvec", Scalar::Bool),
        _ => return None,
    };
    Some(ty)
}

/// `matN` and `matCxR`.
fn matrix(name: &str) -> Option<Ty> {
    let dims = name.strip_prefix("mat")?;
    let (cols, rows) = match dims.split_once('x') {
        Some((cols, rows)) => (cols.parse::<u8>().ok()?, rows.parse::<u8>().ok()?),
        None => {
            let n = dims.parse::<u8>().ok()?;
            (n, n)
        }
    };
    ((2..=4).contains(&cols) && (2..=4).contains(&rows)).then_some(Ty::Num {
        scalar: Scalar::Float,
        rows,
        cols,
    })
}

/// Words that qualify a declaration and can be skipped by the pass.
pub(super) fn is_qualifier(word: &str) -> bool {
    matches!(
        word,
        "const"
            | "uniform"
            | "in"
            | "out"
            | "inout"
            | "highp"
            | "mediump"
            | "lowp"
            | "flat"
            | "smooth"
            | "noperspective"
            | "centroid"
            | "invariant"
            | "precise"
    )
}

/// How a built-in function treats integer arguments and what it returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Builtin {
    /// Float-only, component-wise (`sin`, `pow`, `mix`...): every integer
    /// argument is converted; the result has the widest argument's shape.
    FloatWise,
    /// Has integer overloads (`abs`, `min`, `clamp`...): integer arguments
    /// are converted only when another argument is a float.
    MixedWise,
    /// Returns a float scalar (`length`, `dot`...); arguments as
    /// [`Builtin::FloatWise`].
    FloatScalar,
    /// `cross`: a `vec3`.
    Cross,
    /// A texture lookup returning `vec4`. The argument at this index, if
    /// any, is a float scalar (the LOD or the bias).
    Texture(Option<usize>),
    /// `texelFetch` and friends: integer arguments, `vec4`.
    Fetch,
    /// `textureSize`: an `ivecN` from the sampler.
    TextureSize,
    /// `any`, `all`: `bool`.
    BoolScalar,
    /// `not`, `lessThan`...: a `bvecN` the shape of the first argument.
    BoolWise,
}

/// The rule for a built-in function name.
pub(super) fn builtin_function(name: &str) -> Option<Builtin> {
    let rule = match name {
        "radians" | "degrees" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh"
        | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "pow" | "exp" | "log" | "exp2"
        | "log2" | "sqrt" | "inversesqrt" | "floor" | "ceil" | "fract" | "mod" | "step"
        | "smoothstep" | "mix" | "trunc" | "round" | "roundEven" | "normalize" | "faceforward"
        | "reflect" | "refract" | "dFdx" | "dFdy" | "fwidth" | "matrixCompMult" | "transpose"
        | "inverse" => Builtin::FloatWise,
        "abs" | "sign" | "min" | "max" | "clamp" => Builtin::MixedWise,
        "length" | "distance" | "dot" | "determinant" => Builtin::FloatScalar,
        "cross" => Builtin::Cross,
        "texture" | "textureProj" => Builtin::Texture(Some(2)),
        "textureLod" | "textureProjLod" => Builtin::Texture(Some(2)),
        "textureGrad" | "textureProjGrad" | "textureOffset" | "textureLodOffset"
        | "textureGradOffset" | "textureProjOffset" => Builtin::Texture(None),
        "texelFetch" | "texelFetchOffset" => Builtin::Fetch,
        "textureSize" => Builtin::TextureSize,
        "any" | "all" => Builtin::BoolScalar,
        "not" | "lessThan" | "lessThanEqual" | "greaterThan" | "greaterThanEqual" | "equal"
        | "notEqual" | "isnan" | "isinf" => Builtin::BoolWise,
        _ => return None,
    };
    Some(rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_parse_to_shapes() {
        assert_eq!(
            builtin_type("vec3"),
            Some(Ty::vector(Scalar::Float, 3)),
            "vec3"
        );
        assert_eq!(
            builtin_type("ivec2"),
            Some(Ty::vector(Scalar::Int, 2)),
            "ivec2"
        );
        assert_eq!(
            builtin_type("mat3x2"),
            Some(Ty::Num {
                scalar: Scalar::Float,
                rows: 2,
                cols: 3
            }),
            "mat3x2 is three columns of two rows"
        );
        assert_eq!(builtin_type("vec5"), None, "no vec5");
        assert_eq!(builtin_type("myStruct"), None, "not a built-in");
    }

    #[test]
    fn only_integer_shapes_with_a_float_twin_have_a_constructor() {
        assert_eq!(Ty::INT.float_constructor(), Some("float"), "int");
        assert_eq!(
            Ty::vector(Scalar::Int, 3).float_constructor(),
            Some("vec3"),
            "ivec3"
        );
        assert_eq!(Ty::Unknown.float_constructor(), None, "unknown");
    }
}
