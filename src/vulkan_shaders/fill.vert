#version 450

#include "fill.common.glsl"

layout(location = 0) out vec2 pos;

void main() {
	Data data = push_data.data;
	switch (gl_VertexIndex) {
		case 0: pos = vec2(data.x2, data.y1); break;
		case 1: pos = vec2(data.x1, data.y1); break;
		case 2: pos = vec2(data.x2, data.y2); break;
		case 3: pos = vec2(data.x1, data.y2); break;
	}
	gl_Position = vec4(pos, 0.0, 1.0);
}
