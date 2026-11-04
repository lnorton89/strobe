// Stripes whose brightness is the smoothed control.
vec4 renderMain(void) {
    float band = step(0.5, fract(_uv.x * stripes));
    return vec4(vec3(band * amount), 1.0);
}
