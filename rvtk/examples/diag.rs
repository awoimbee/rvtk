use vtk::*;

const W: i32 = 200;
const H: i32 = 150;
const BG: [f64; 3] = [0.10, 0.20, 0.35];

fn new_window() -> (vtkRenderWindow, vtkRenderer) {
    let renderer = vtkRenderer::new();
    renderer.set_background(BG);
    let window = vtkRenderWindow::new();
    window.add_renderer(&renderer);
    let mut size = [W, H];
    window.set_off_screen_rendering(1);
    window.set_multi_samples(0);
    window.set_size(&mut size);
    (window, renderer)
}

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
    mapper.set_input_data(&source.get_output().expect("out"));
    let actor = vtkActor::new();
    actor.set_mapper(&mapper);
    renderer.add_actor(&actor);
    (source, mapper, actor)
}

fn capture(window: &vtkRenderWindow) -> Vec<[f32; 4]> {
    window.render();
    let to_image = vtkWindowToImageFilter::new();
    to_image.set_input(window);
    to_image.set_input_buffer_type_to_rgba();
    to_image.set_should_rerender(0);
    to_image.update();
    let image = to_image.get_output().expect("out");
    let image = vtkImageData::safe_down_cast(&image).expect("image");
    let mut dims = [0i32; 3];
    image.get_dimensions(&mut dims);
    let mut v = Vec::new();
    for y in 0..dims[1] {
        for x in 0..dims[0] {
            v.push([
                image.get_scalar_component_as_float(x, y, 0, 0),
                image.get_scalar_component_as_float(x, y, 0, 1),
                image.get_scalar_component_as_float(x, y, 0, 2),
                image.get_scalar_component_as_float(x, y, 0, 3),
            ]);
        }
    }
    v
}

fn diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> usize {
    let tol = 6.0 / 255.0;
    a.iter()
        .zip(b)
        .filter(|(x, y)| (0..4).any(|c| (x[c] - y[c]).abs() > tol))
        .count()
}

fn main() {
    let (w0, r0) = new_window();
    r0.reset_camera();
    let reference = capture(&w0);

    // one sphere
    let (w1, r1) = new_window();
    let _s1 = add_sphere(&r1, 0.5);
    r1.reset_camera();
    let one = capture(&w1);
    println!("one vs ref: {}", diff(&one, &reference));

    // two spheres
    let (w2, r2) = new_window();
    let _a = add_sphere(&r2, 0.5);
    r2.reset_camera();
    let (_s2, _m2, actor2) = add_sphere(&r2, 0.5);
    actor2.set_position_v2(1.0, 0.0, 0.0);
    r2.reset_camera();
    let two = capture(&w2);
    println!("two vs ref: {}", diff(&two, &reference));
    println!("two vs one: {}", diff(&two, &one));

    // unique RGB in one
    let mut cols: Vec<[u8; 3]> = one
        .iter()
        .map(|p| [p[0] as u8, p[1] as u8, p[2] as u8])
        .collect();
    cols.sort_unstable();
    cols.dedup();
    println!("unique RGB (raw cast) in one: {}", cols.len());
}
