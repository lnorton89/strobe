// A left-to-right wipe with a soft edge, in the GL Transitions shape:
// define transition(uv) and use getFromColor / getToColor / progress.
vec4 transition(vec2 uv) {
    float edge = 0.08;
    float m = smoothstep(progress - edge, progress, 1.0 - uv.x);
    return mix(getFromColor(uv), getToColor(uv), 1.0 - m);
}
