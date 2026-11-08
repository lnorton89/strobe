// Looks all the way round the cube map: longitude across, latitude up,
// so every face shows somewhere in the frame.
vec4 renderMain(void) {
    float lon = _uv.x * 6.2831853;
    float lat = (_uv.y - 0.5) * 3.1415927;
    vec3 dir = vec3(cos(lat) * cos(lon), sin(lat), cos(lat) * sin(lon));
    return vec4(texture(room, dir).rgb, 1.0);
}
