// `stripes` is defined by nothing here: it compiles only if the include
// beside this file is expanded when the scene loads.
#include "helpers.glsl"

vec4 renderMain(void) {
    return vec4(stripes(_uv, 7.0), 0.3, 1.0 - stripes(_uv.yx, 3.0), 1.0);
}
