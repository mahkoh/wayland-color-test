#extension GL_EXT_buffer_reference : require

layout(buffer_reference, buffer_reference_align = 16, row_major, std430) readonly buffer Data {
	mat4x4 lms_to_local;
	float x1;
	float y1;
	float x2;
	float y2;
	vec4 color[4];
	uint oetf;
};

layout(push_constant, std430) uniform PushData {
	Data data;
} push_data;
