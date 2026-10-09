// ---- Strobe clock pass (generated) ----
// One float texel: x = iTime, y = last TIME, z = real dt, w = start frame + 1.
float strobe_rate() {
    return st_speed * mix(1.0, 0.3 + 2.0 * syn_BassLevel, st_audio_time);
}
vec4 renderMain() {
    vec4 prev = texelFetch(st_clock, ivec2(0, 0), 0);
    bool fresh = prev.w == 0.0 || TIME < prev.y;
    float dt = fresh ? 0.0 : TIME - prev.y;
    float clock = fresh ? TIME : prev.x + dt * strobe_rate();
    float start = fresh ? FRAMECOUNT + 1.0 : prev.w;
    return vec4(clock, TIME, dt, start);
}
