// `tint` is declared by nothing but the script: it compiles only if the
// script ran before the shader was built.
vec4 renderMain(void) {
    return vec4(tint * (0.5 + 0.5 * _uv.x), 1.0);
}
