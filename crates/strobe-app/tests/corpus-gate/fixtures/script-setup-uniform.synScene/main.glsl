// `level` comes from the script setup().
vec4 renderMain(void) {
    return vec4(level, 1.0 - level, _uv.y, 1.0);
}
