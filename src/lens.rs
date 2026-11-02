//! A real magnifying lens: copies the pixels already drawn under it and redraws them enlarged.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, PaintCallbackInfo, Painter, Pos2, Rect, Vec2};
use eframe::egui_glow;
use eframe::glow::{self, HasContext as _};

#[derive(Clone, Default)]
pub struct Lens {
    gl: Arc<Mutex<GlState>>,
}

#[derive(Default)]
enum GlState {
    #[default]
    NotYet,
    Ready(LensGl),
    /// Shaders didn't compile on this GPU; the glass then just shows a tint.
    Failed,
}

struct LensGl {
    program: glow::Program,
    vao: glow::VertexArray,
    _vbo: glow::Buffer,
    texture: glow::Texture,
    u_tex: Option<glow::UniformLocation>,
    u_vp_origin: Option<glow::UniformLocation>,
    u_vp_size: Option<glow::UniformLocation>,
    u_center: Option<glow::UniformLocation>,
    u_radius: Option<glow::UniformLocation>,
    u_zoom: Option<glow::UniformLocation>,
    u_cap_origin: Option<glow::UniformLocation>,
    u_cap_size: Option<glow::UniformLocation>,
}

impl Lens {
    /// Magnifies whatever has been painted under the circle so far this frame.
    pub fn paint(&self, painter: &Painter, center: Pos2, radius: f32, zoom: f32) {
        if radius < 2.0 {
            return;
        }
        let rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0 + 2.0));
        let state = self.gl.clone();
        let callback = egui_glow::CallbackFn::new(move |info, painter| {
            let gl = painter.gl();
            let mut state = state.lock().unwrap();
            if matches!(*state, GlState::NotYet) {
                *state = match unsafe { LensGl::new(gl) } {
                    Ok(lens) => GlState::Ready(lens),
                    Err(e) => {
                        eprintln!("magnifying lens disabled: {e}");
                        GlState::Failed
                    }
                };
            }
            if let GlState::Ready(lens) = &*state {
                unsafe { lens.draw(gl, &info, center, radius, zoom) };
            }
        });
        painter.add(egui::PaintCallback { rect, callback: Arc::new(callback) });
    }

}

const VERTEX: &str = r#"
#ifdef NEW_SHADER_INTERFACE
in vec2 a_pos;
out vec2 v_px;
#else
attribute vec2 a_pos;
varying vec2 v_px;
#endif
uniform vec2 u_vp_origin;
uniform vec2 u_vp_size;
void main() {
    v_px = u_vp_origin + a_pos * u_vp_size;
    gl_Position = vec4(a_pos * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"
#ifdef GL_ES
precision mediump float;
#endif
#ifdef NEW_SHADER_INTERFACE
in vec2 v_px;
out vec4 f_color;
#define TEX texture
#else
varying vec2 v_px;
#define TEX texture2D
#endif
uniform sampler2D u_tex;
uniform vec2 u_center;
uniform float u_radius;
uniform float u_zoom;
uniform vec2 u_cap_origin;
uniform vec2 u_cap_size;

vec3 grab(vec2 px) {
    vec2 uv = (px - u_cap_origin) / u_cap_size;
    return TEX(u_tex, clamp(uv, vec2(0.001), vec2(0.999))).rgb;
}

void main() {
    vec2 d = v_px - u_center;
    float r = length(d) / u_radius;
    if (r > 1.0) discard;
    // Strongest in the middle, easing off towards the rim like a real convex lens.
    float k = (1.0 + 0.45 * r * r) / u_zoom;
    vec2 src = u_center + d * k;
    vec2 dir = d / max(length(d), 0.001);
    float fringe = 0.015 * r * r * u_radius;
    vec3 col = vec3(grab(src + dir * fringe).r, grab(src).g, grab(src - dir * fringe).b);
    col = mix(col, vec3(0.62, 0.80, 0.93), 0.05 + 0.13 * r * r);
    col *= 1.0 - 0.22 * smoothstep(0.72, 1.0, r);
    // Window-light glint in the upper left (GL's y axis points up).
    vec2 g = (v_px - (u_center + vec2(-0.36, 0.40) * u_radius)) / u_radius;
    col += vec3(1.0) * exp(-dot(g * vec2(1.0, 2.2), g * vec2(1.0, 2.2)) * 22.0) * 0.55;
    float a = 1.0 - smoothstep(0.965, 1.0, r);
#ifdef NEW_SHADER_INTERFACE
    f_color = vec4(col * a, a);
#else
    gl_FragColor = vec4(col * a, a);
#endif
}
"#;

impl LensGl {
    unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let version = egui_glow::ShaderVersion::get(gl);
            let header = format!(
                "{}\n{}\n",
                version.version_declaration(),
                if version.is_new_shader_interface() { "#define NEW_SHADER_INTERFACE" } else { "" }
            );
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)] {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, &format!("{header}{src}"));
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.bind_attrib_location(program, 0, "a_pos");
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }

            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            let bytes: Vec<u8> = quad.iter().flat_map(|f| f.to_ne_bytes()).collect();
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);

            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }

            let u = |name| gl.get_uniform_location(program, name);
            Ok(Self {
                u_tex: u("u_tex"),
                u_vp_origin: u("u_vp_origin"),
                u_vp_size: u("u_vp_size"),
                u_center: u("u_center"),
                u_radius: u("u_radius"),
                u_zoom: u("u_zoom"),
                u_cap_origin: u("u_cap_origin"),
                u_cap_size: u("u_cap_size"),
                program,
                vao,
                _vbo: vbo,
                texture,
            })
        }
    }

    unsafe fn draw(&self, gl: &glow::Context, info: &PaintCallbackInfo, center: Pos2, radius: f32, zoom: f32) {
        let ppp = info.pixels_per_point;
        let [screen_w, screen_h] = [info.screen_size_px[0] as i32, info.screen_size_px[1] as i32];
        let vp = info.viewport_in_pixels();
        // Only the part of the lens that's on screen can be copied.
        let x0 = vp.left_px.max(0);
        let y0 = vp.from_bottom_px.max(0);
        let x1 = (vp.left_px + vp.width_px).min(screen_w);
        let y1 = (vp.from_bottom_px + vp.height_px).min(screen_h);
        if x1 - x0 < 2 || y1 - y0 < 2 {
            return;
        }
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.copy_tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA, x0, y0, x1 - x0, y1 - y0, 0);

            gl.use_program(Some(self.program));
            gl.uniform_1_i32(self.u_tex.as_ref(), 0);
            gl.uniform_2_f32(self.u_vp_origin.as_ref(), vp.left_px as f32, vp.from_bottom_px as f32);
            gl.uniform_2_f32(self.u_vp_size.as_ref(), vp.width_px as f32, vp.height_px as f32);
            gl.uniform_2_f32(self.u_center.as_ref(), center.x * ppp, screen_h as f32 - center.y * ppp);
            gl.uniform_1_f32(self.u_radius.as_ref(), radius * ppp);
            gl.uniform_1_f32(self.u_zoom.as_ref(), zoom);
            gl.uniform_2_f32(self.u_cap_origin.as_ref(), x0 as f32, y0 as f32);
            gl.uniform_2_f32(self.u_cap_size.as_ref(), (x1 - x0) as f32, (y1 - y0) as f32);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
        }
    }
}
