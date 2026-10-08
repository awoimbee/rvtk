//! Rendering tests: build a real scene, render it offscreen and inspect the
//! resulting pixels.
//!
//! These exercise a much larger slice of VTK than a compute-only pipeline: the
//! OpenGL backend, actor/mapper/property state, the camera, and the whole
//! `Deref` inheritance chain (`vtkRenderer` -> `vtkViewport`,
//! `vtkRenderWindow` -> `vtkWindow`, `vtkActor` -> `vtkProp3D` -> `vtkProp`, ...).
//!
//! # Why this file has its own `main`
//!
//! This target uses `harness = false` (see `Cargo.toml`).  VTK's Cocoa backend
//! creates an `NSWindow` on the first `Render()` *even in offscreen mode*
//! (`vtkCocoaRenderWindow::Start` always calls `CreateAWindow`), and AppKit only
//! permits that on the main thread.  The default libtest harness spawns a worker
//! thread per test, which makes AppKit raise and abort the process.  Running the
//! checks from `main` keeps them on the main thread.
//!
//! Every check writes a PNG to `rvtk/target/rvtk-test-images/` so failures can
//! be inspected by eye.

use std::path::PathBuf;
use std::process::ExitCode;

use vtk::*;

const WIDTH: i32 = 200;
const HEIGHT: i32 = 150;

/// Two visibly different colours, to tell "background" from "object".
const BACKGROUND: [f64; 3] = [0.10, 0.20, 0.35];

/// Channel distance (in 0..1 units) below which two pixels count as equal.
/// The framebuffer goes through 8-bit quantisation, so exact equality is too
/// strict.
const TOLERANCE: f32 = 6.0 / 255.0;

// ---------------------------------------------------------------------------
// scene helpers
// ---------------------------------------------------------------------------

/// An offscreen render window with a renderer attached.
fn new_window() -> (vtkRenderWindow, vtkRenderer) {
    let renderer = vtkRenderer::new();
    // `SetBackground` lives on `vtkViewport`, reached through `Deref`.
    renderer.set_background(BACKGROUND);

    let window = vtkRenderWindow::new();
    window.add_renderer(&renderer);
    let mut size = [WIDTH, HEIGHT];

    // `SetOffScreenRendering`/`SetSize` live on `vtkWindow`, also via `Deref`.
    window.set_off_screen_rendering(1);
    window.set_multi_samples(0); // keep pixel comparisons crisp
    window.set_size(&mut size);

    (window, renderer)
}

/// Add a sphere to `renderer` through the usual source -> mapper -> actor chain.
fn add_sphere(
    renderer: &vtkRenderer,
    radius: f64,
) -> (vtkSphereSource, vtkPolyDataMapper, vtkActor) {
    let source = vtkSphereSource::new();
    source.set_radius(radius);
    source.set_theta_resolution(32);
    source.set_phi_resolution(32);
    source.update();

    let mapper = vtkPolyDataMapper::new();
    mapper.set_input_data(&source.get_output().expect("sphere produced output"));

    let actor = vtkActor::new();
    actor.set_mapper(&mapper);
    renderer.add_actor(&actor);

    (source, mapper, actor)
}

/// Render `window` and read it back through `vtkWindowToImageFilter`.
fn capture(window: &vtkRenderWindow) -> vtkImageData {
    window.render();

    let to_image = vtkWindowToImageFilter::new();
    to_image.set_input(window);
    to_image.set_input_buffer_type_to_rgba();
    to_image.set_should_rerender(0);
    to_image.update();

    let image = to_image
        .get_output()
        .expect("window to image produced output");
    // The filter keeps the output alive through the pipeline; take our own
    // reference so the image outlives `to_image`.
    vtkImageData::safe_down_cast(&image).expect("output is an image")
}

/// Dimensions plus RGBA samples of an image.
struct Image {
    width: i32,
    height: i32,
    rgba: Vec<[f32; 4]>,
}

impl Image {
    /// Pixel at (`x`, `y`), counting from the bottom-left as VTK does.
    fn at(&self, x: i32, y: i32) -> [f32; 4] {
        self.rgba[(y * self.width + x) as usize]
    }

    /// Number of pixels that differ from the corresponding pixel of `other`.
    fn diff_count(&self, other: &Image) -> usize {
        assert_eq!(self.rgba.len(), other.rgba.len());
        self.rgba
            .iter()
            .zip(&other.rgba)
            .filter(|(a, b)| (0..4).any(|c| (a[c] - b[c]).abs() > TOLERANCE))
            .count()
    }

    /// True when every pixel has the same colour.
    fn is_uniform(&self) -> bool {
        let first = self.rgba[0];
        self.rgba
            .iter()
            .all(|p| (0..4).all(|c| (p[c] - first[c]).abs() <= TOLERANCE))
    }

    /// Average colour of the pixels that differ from `other`.
    fn mean_difference_color(&self, other: &Image) -> [f32; 3] {
        let mut sum = [0.0f32; 3];
        let mut n = 0usize;
        for (a, b) in self.rgba.iter().zip(&other.rgba) {
            if (0..4).any(|c| (a[c] - b[c]).abs() > TOLERANCE) {
                for c in 0..3 {
                    sum[c] += a[c];
                }
                n += 1;
            }
        }
        if n == 0 {
            return [0.0; 3];
        }
        [sum[0] / n as f32, sum[1] / n as f32, sum[2] / n as f32]
    }
}

fn read_pixels(image: &vtkImageData) -> Image {
    let mut dims = [0i32; 3];
    image.get_dimensions(&mut dims);
    let (width, height) = (dims[0], dims[1]);
    assert!(width > 0 && height > 0, "image has no extent: {dims:?}");

    let mut rgba = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.push([
                image.get_scalar_component_as_float(x, y, 0, 0),
                image.get_scalar_component_as_float(x, y, 0, 1),
                image.get_scalar_component_as_float(x, y, 0, 2),
                image.get_scalar_component_as_float(x, y, 0, 3),
            ]);
        }
    }
    Image {
        width,
        height,
        rgba,
    }
}

/// Render a scene with no actors.  Used as a reference so tests do not have to
/// guess how VTK maps `SetBackground` onto framebuffer values.
fn empty_scene_image() -> Image {
    let (window, renderer) = new_window();
    renderer.reset_camera();
    read_pixels(&capture(&window))
}

fn artifact_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("rvtk-test-images");
    std::fs::create_dir_all(&dir).expect("create artifact directory");
    dir.join(name)
}

/// Write `image` to `rvtk/target/rvtk-test-images/<name>.png`.
fn save_png(image: &vtkImageData, name: &str) -> PathBuf {
    let path = artifact_path(name);
    let writer = vtkPNGWriter::new();
    writer.set_file_name(&path.to_string_lossy());
    writer.set_input_data(image);
    writer.write();
    assert!(
        path.exists(),
        "vtkPNGWriter did not create {}",
        path.display()
    );
    path
}

// ---------------------------------------------------------------------------
// checks
// ---------------------------------------------------------------------------

fn empty_scene_is_a_uniform_background() {
    let reference = empty_scene_image();
    assert_eq!((reference.width, reference.height), (WIDTH, HEIGHT));
    assert!(
        reference.is_uniform(),
        "an empty scene should be a single flat colour"
    );
    // Opaque.
    assert!(
        reference.rgba.iter().all(|p| p[3] > 0.99),
        "alpha not opaque"
    );

    let bg = reference.at(0, 0);
    println!("      background = {bg:?}");
}

fn renders_a_shaded_sphere() {
    let reference = empty_scene_image();

    let (window, renderer) = new_window();
    let _sphere = add_sphere(&renderer, 0.7);
    renderer.reset_camera();

    let image = capture(&window);
    let px = read_pixels(&image);

    assert_eq!((px.width, px.height), (WIDTH, HEIGHT));
    assert_eq!(window.get_never_rendered(), 0, "window never rendered");

    let painted = px.diff_count(&reference);
    let total = px.rgba.len();
    assert!(
        painted > total / 10,
        "sphere covered only {painted}/{total} pixels"
    );
    assert!(
        painted < total * 9 / 10,
        "sphere covered {painted}/{total} pixels; is the background visible?"
    );

    // Shading: the object is not one flat colour.
    let mut colors: Vec<[u8; 3]> = px
        .rgba
        .iter()
        .map(|p| [p[0], p[1], p[2]].map(|c| (c * 255.0) as u8))
        .collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(
        colors.len() > 10,
        "expected a shaded sphere, got {} colours",
        colors.len()
    );

    let path = save_png(&image, "sphere_offscreen.png");
    println!("      {} painted pixels -> {}", painted, path.display());
}

fn camera_orientation_changes_the_image() {
    let (window, renderer) = new_window();
    let _sphere = add_sphere(&renderer, 0.7);

    let camera = renderer.get_active_camera().expect("renderer has a camera");

    // `Azimuth` rotates the camera about its focal point.
    camera.azimuth(0.0);
    renderer.reset_camera();
    let front = read_pixels(&capture(&window));

    camera.azimuth(90.0);
    renderer.reset_camera();
    let side = read_pixels(&capture(&window));

    let differing = front.diff_count(&side);
    assert!(
        differing > 100,
        "camera azimuth had almost no effect ({differing} pixels differ)"
    );
    println!("      {differing} pixels differ between azimuth 0 and 90");
}

fn actor_colour_is_visible_in_the_image() {
    let reference = empty_scene_image();

    let (window, renderer) = new_window();
    let (_source, _mapper, actor) = add_sphere(&renderer, 0.7);

    // A saturated red actor with no ambient or specular term, so the painted
    // pixels should be strongly red-dominant.
    let property = actor.get_property().expect("actor has a property");
    property.set_color_v2(1.0, 0.0, 0.0);
    property.set_ambient(0.0);
    property.set_diffuse(1.0);
    property.set_specular(0.0);

    renderer.reset_camera();

    let image = capture(&window);
    let px = read_pixels(&image);
    let mean = px.mean_difference_color(&reference);

    assert!(
        mean[0] > mean[1] + 0.05 && mean[0] > mean[2] + 0.05,
        "expected a red-dominant image, got {mean:?}"
    );

    save_png(&image, "sphere_red.png");
    println!("      mean painted colour = {mean:?}");
}

fn two_actors_paint_more_than_one() {
    let reference = empty_scene_image();

    let (window, renderer) = new_window();
    let _first = add_sphere(&renderer, 0.5);
    renderer.reset_camera();
    let one = read_pixels(&capture(&window)).diff_count(&reference);

    let (_source, _mapper, actor) = add_sphere(&renderer, 0.5);
    actor.set_position_v2(1.0, 0.0, 0.0);
    renderer.reset_camera();
    let two = read_pixels(&capture(&window)).diff_count(&reference);

    assert!(
        two > one,
        "adding a second actor did not add pixels ({one} -> {two})"
    );
    println!("      {one} painted pixels with one actor, {two} with two");
}

fn png_writer_emits_a_valid_png() {
    let (window, renderer) = new_window();
    let _sphere = add_sphere(&renderer, 0.7);
    renderer.reset_camera();

    let image = capture(&window);
    let path = save_png(&image, "sphere_writer.png");

    let bytes = std::fs::read(&path).expect("read the png back");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG file");
    assert!(
        bytes.len() > 500,
        "png is suspiciously small ({} bytes); did anything render?",
        bytes.len()
    );
    println!("      wrote {} ({} bytes)", path.display(), bytes.len());
}

fn render_window_reports_a_backend() {
    let (window, renderer) = new_window();
    let _sphere = add_sphere(&renderer, 0.5);
    renderer.reset_camera();
    window.render();

    let backend = window.get_rendering_backend();
    assert!(!backend.is_empty(), "no rendering backend reported");
    println!("      backend = {backend}");
}

// ---------------------------------------------------------------------------
// runner
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let checks: &[(&str, fn())] = &[
        (
            "empty_scene_is_a_uniform_background",
            empty_scene_is_a_uniform_background,
        ),
        ("renders_a_shaded_sphere", renders_a_shaded_sphere),
        (
            "camera_orientation_changes_the_image",
            camera_orientation_changes_the_image,
        ),
        (
            "actor_colour_is_visible_in_the_image",
            actor_colour_is_visible_in_the_image,
        ),
        (
            "two_actors_paint_more_than_one",
            two_actors_paint_more_than_one,
        ),
        ("png_writer_emits_a_valid_png", png_writer_emits_a_valid_png),
        (
            "render_window_reports_a_backend",
            render_window_reports_a_backend,
        ),
    ];

    println!("running {} rendering checks", checks.len());
    let mut failed = 0usize;

    for (name, check) in checks {
        print!("  {name} ... ");
        // `catch_unwind` turns a failed assertion into a reported failure
        // instead of aborting the whole run.  The checks are deliberately run
        // sequentially on the main thread (see the module docs).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check()));
        match result {
            Ok(()) => println!("ok"),
            Err(_) => {
                println!("FAILED");
                failed += 1;
            }
        }
    }

    if failed == 0 {
        println!("all {0} rendering checks passed", checks.len());
        ExitCode::SUCCESS
    } else {
        eprintln!("{failed} of {} rendering checks failed", checks.len());
        ExitCode::FAILURE
    }
}
