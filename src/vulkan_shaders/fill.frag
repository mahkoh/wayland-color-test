#version 450

#include "fill.common.glsl"

#define TF_LINEAR 1
#define TF_ST2084_PQ 2
#define TF_BT1886 3
#define TF_GAMMA22 4
#define TF_GAMMA28 5
#define TF_ST240 6
#define TF_LOG100 8
#define TF_LOG316 9
#define TF_ST428 10
#define TF_POW 11

vec3 inv_eotf_bt1886(Data data, vec3 c) {
	c = clamp(c, 0.0, 1.0);
	float a1 = data.eotf_arg1;
	float a2 = data.eotf_arg2;
	float a3 = data.eotf_arg3;
	float a4 = data.eotf_arg4;
	return a1 * (pow(a2 * c + a3, vec3(1.0 / 2.4)) - a4);
}

vec3 inv_eotf_st2084_pq(vec3 c) {
	c = clamp(c, 0.0, 1.0);
	vec3 num = vec3(0.8359375) + vec3(18.8515625) * pow(c, vec3(0.1593017578125));
	vec3 den = vec3(1.0) + vec3(18.6875) * pow(c, vec3(0.1593017578125));
	return pow(num / den, vec3(78.84375));
}

vec3 inv_eotf_st240(vec3 c) {
	return mix(
		vec3(4.0) * c,
		vec3(1.1115) * pow(c, vec3(0.45)) - vec3(0.1115),
		greaterThanEqual(c, vec3(0.0228))
	);
}

vec3 inv_eotf_log100(vec3 c) {
	c = clamp(c, 0.0, 1.0);
	return mix(
		vec3(0.0),
		vec3(1.0) + log2(c) / vec3(log2(10)) / vec3(2.0),
		greaterThanEqual(c, vec3(0.01))
	);
}

vec3 inv_eotf_log316(vec3 c) {
	c = clamp(c, 0.0, 1.0);
	return mix(
		vec3(0.0),
		vec3(1.0) + log2(c) / vec3(log2(10)) / vec3(2.5),
		greaterThanEqual(c, vec3(sqrt(10) / 1000.0))
	);
}

vec3 inv_eotf_st428(vec3 c) {
	c = max(c, 0.0);
	return pow(vec3(48.0) * c / vec3(52.37), vec3(1.0 / 2.6));
}

vec3 apply_inv_eotf(Data data, vec3 c) {
	switch (data.eotf) {
		case TF_LINEAR: return c;
		case TF_ST2084_PQ: return inv_eotf_st2084_pq(c);
		case TF_BT1886: return inv_eotf_bt1886(data, c);
		case TF_GAMMA22: return sign(c) * pow(abs(c), vec3(1.0 / 2.2));
		case TF_GAMMA28: return sign(c) * pow(abs(c), vec3(1.0 / 2.8));
		case TF_ST240: return inv_eotf_st240(c);
		case TF_LOG100: return inv_eotf_log100(c);
		case TF_LOG316: return inv_eotf_log316(c);
		case TF_ST428: return inv_eotf_st428(c);
		case TF_POW: return sign(c) * pow(abs(c), vec3(data.eotf_arg1));
		default: return c;
	}
}

const mat3 LAB_TO_LMS_PRIME = mat3(
	1.0,           1.0,           1.0,
	0.3963377774, -0.1055613458, -0.0894841775,
	0.2158037573, -0.0638541728, -1.2914855480
);

layout(location = 0) in vec2 pos;
layout(location = 0) out vec4 out_color;

void main() {
	Data data = push_data.data;
	float x_factor = (pos.x - data.x1) / (data.x2 - data.x1);
	float y_factor = (pos.y - data.y1) / (data.y2 - data.y1);
	vec4 color =          y_factor  * (x_factor * data.color[2] + (1 - x_factor) * data.color[3])
				 + (1.0 - y_factor) * (x_factor * data.color[0] + (1 - x_factor) * data.color[1]);
	vec3 c = color.rgb;
	c = LAB_TO_LMS_PRIME * c;
	c = c * c * c;
	c = (data.lms_to_local * vec4(c, 1.0)).rgb;
	c = apply_inv_eotf(data, c);
	out_color = vec4(c, color.a);
}
