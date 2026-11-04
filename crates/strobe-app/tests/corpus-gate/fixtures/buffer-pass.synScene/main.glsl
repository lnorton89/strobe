// Pass 0 writes a field; the final pass reads it back, coloured.
vec4 renderMain(void) {
    if (PASSINDEX == 0) {
        return vec4(fract(_uv * 3.0), 0.0, 1.0);
    }
    vec4 f = texture(field, _uv);
    return vec4(f.r, f.g * 0.5, 1.0 - f.r, 1.0);
}
