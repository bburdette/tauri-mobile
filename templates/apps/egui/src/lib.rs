use std::iter;
use std::time::Instant;

use epi::*;
use winit::event_loop::ControlFlow;
use winit::event::{Event::*};
use egui_wgpu_backend::{RenderPass, ScreenDescriptor};

use mobile_entry_point::mobile_entry_point;
#[cfg(target_os = "android")]
use ndk_glue;

/// A custom event type for the winit app.
#[derive(Debug)]
enum Event {
    RequestRedraw,
}

/// This is the repaint signal type that egui needs for requesting a repaint from another thread.
/// It sends the custom RequestRedraw event to the winit event loop.
struct ExampleRepaintSignal(std::sync::Mutex<winit::event_loop::EventLoopProxy<Event>>);

impl epi::RepaintSignal for ExampleRepaintSignal {
    fn request_repaint(&self) {
        self.0.lock().unwrap().send_event(Event::RequestRedraw).ok();
    }
}

#[cfg(target_os = "android")]
fn wait_for_native_screen() {
    log::info!("App started. Waiting for NativeScreen");
    loop {
        match ndk_glue::native_window().as_ref() {
            Some(_) => {
                log::info!("NativeScreen Found:{:?}", ndk_glue::native_window());
                break;
            }
            None => (),
        }
    }
}

/// A simple egui + wgpu + winit based example.
#[mobile_entry_point]
fn main() {
    let event_loop = winit::event_loop::EventLoop::with_user_event();
    let window = winit::window::WindowBuilder::new()
        .with_decorations(true)
        .with_transparent(false)
        .with_title("A fantastic window!")
        .with_inner_size(winit::dpi::PhysicalSize {
            width: 1280.0,
            height: 720.0,
        })
        .build(&event_loop)
        .unwrap();

    #[cfg(target_os = "android")]
    wait_for_native_screen();

    let instance = wgpu::Instance::new(wgpu::Backends::PRIMARY);

    let surface = unsafe { instance.create_surface(&window) };

    // WGPU 0.11+ support force fallback (if HW implementation not supported), set it to true or false (optional).
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .unwrap();

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            features: wgpu::Features::default(),
            limits: wgpu::Limits::default(),
            label: None,
        },
        None,
    ))
    .unwrap();

    let size = window.inner_size();
    let surface_format = surface.get_preferred_format(&adapter).unwrap();
    let mut surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width as u32,
        height: size.height as u32,
        present_mode: wgpu::PresentMode::Mailbox,
    };
    surface.configure(&device, &surface_config);

    let repaint_signal = std::sync::Arc::new(ExampleRepaintSignal(std::sync::Mutex::new(
        event_loop.create_proxy(),
    )));


    // We use the egui_wgpu_backend crate as the render backend.
    let mut egui_rpass = RenderPass::new(&device, surface_format, 1);

    // Display the demo application that ships with egui.
    let mut demo_app = egui_demo_lib::WrapApp::default();

    let mut previous_frame_time = None;

    let mut state = egui_winit::State::new(&window);
    let mut ctx = egui::CtxRef::default();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        let mut redraw = || {
            let output_frame = match surface.get_current_texture() {
                Ok(frame) => frame,
                Err(e) => {
                    eprintln!("Dropped frame with error: {}", e);
                    return;
                }
            };
            let output_view = output_frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            let egui_start = Instant::now();
            let raw_input: egui::RawInput = state.take_egui_input(&window);
            ctx.begin_frame(raw_input);
            let mut app_output = epi::backend::AppOutput::default();

            let mut frame = epi::backend::FrameBuilder {
                info: epi::IntegrationInfo {
                    name: "egui_winit",
                    web_info: None,
                    cpu_usage: previous_frame_time,
                    native_pixels_per_point: Some(window.scale_factor() as _),
                    prefer_dark_mode: None,
                },
                tex_allocator: &mut egui_rpass,
                output: &mut app_output,
                repaint_signal: repaint_signal.clone(),
            }
            .build();

            // Draw the demo application.
            demo_app.update(&ctx, &mut frame);

            // End the UI frame. We could now handle the output and draw the UI with the backend.
            let (_output, paint_commands) = ctx.end_frame();
            let paint_jobs = ctx.tessellate(paint_commands);

            let frame_time = (Instant::now() - egui_start).as_secs_f64() as f32;
            previous_frame_time = Some(frame_time);

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });

            // Upload all resources for the GPU.
            let screen_descriptor = ScreenDescriptor {
                physical_width: surface_config.width,
                physical_height: surface_config.height,
                scale_factor: window.scale_factor() as f32,
            };
            egui_rpass.update_texture(&device, &queue, &ctx.texture());
            egui_rpass.update_user_textures(&device, &queue);
            egui_rpass.update_buffers(&device, &queue, &paint_jobs, &screen_descriptor);

            // Record all render passes.
            egui_rpass
                .execute(
                    &mut encoder,
                    &output_view,
                    &paint_jobs,
                    &screen_descriptor,
                    Some(wgpu::Color::BLACK),
                )
                .unwrap();
            // Submit the commands.
            queue.submit(iter::once(encoder.finish()));

            // Redraw egui
            output_frame.present();
        };
        match event {
            RedrawRequested(..) | UserEvent(Event::RequestRedraw) | MainEventsCleared => {
                redraw();
            }
            WindowEvent { event, .. } => {
                match event {
                    winit::event::WindowEvent::Resized(size) => {
                        surface_config.width = size.width;
                        surface_config.height = size.height;
                        surface.configure(&device, &surface_config);
                    },
                    winit::event::WindowEvent::CloseRequested => {
                        *control_flow = ControlFlow::Exit;
                    },
                    _ => {
                        state.on_event(&ctx, &event);
                    }
                };
            },
            _ => (),
        }
    });
}
