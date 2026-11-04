// A gradient across the frame and a ring at its centre, the same every frame.
vec4 renderMain(void) {
    float ring = smoothstep(0.02, 0.0, abs(length(_uvc) - 0.25));
    return vec4(_uv.x, _uv.y, 0.5 + 0.5 * ring, 1.0);
}
