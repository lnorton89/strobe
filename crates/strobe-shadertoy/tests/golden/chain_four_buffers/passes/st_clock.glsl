// ---- Strobe clock pass (generated) ----
// Pass 0, st_clock: x = iTime, y = last TIME, z = real dt, w = start frame + 1.
// Pass 1, st_frame: x = the frame iFrame counts from, plus one.
float strobe_rate() {
    return st_speed * mix(1.0, 0.3 + 2.0 * syn_BassLevel, st_audio_time);
}
vec4 renderMain() {
    if (PASSINDEX == 0) {
        vec4 prev = texelFetch(st_clock, ivec2(0, 0), 0);
        bool fresh = prev.w == 0.0 || TIME < prev.y;
        float dt = fresh ? 0.0 : TIME - prev.y;
        float clock = fresh ? TIME : prev.x + dt * strobe_rate();
        float start = fresh ? FRAMECOUNT + 1.0 : prev.w;
        return vec4(clock, TIME, dt, start);
    }
    float now = FRAMECOUNT + 1.0;
    float prev = texelFetch(st_frame, ivec2(0, 0), 0).x;
    bool fresh = prev == 0.0 || prev > now || texelFetch(st_clock, ivec2(0, 0), 0).w == now;
    return vec4(fresh ? now : prev, 0.0, 0.0, 1.0);
}
