//! A compute-only version of the classic VTK `SphereSource` example.
//!
//! Run with:
//!
//! ```text
//! cargo run -p vtk-wrap --example sphere_source --features vtkFiltersSources
//! ```

use vtk_wrap::vtkSphereSource;

fn main() {
    let sphere = vtkSphereSource::new();
    sphere.set_radius(2.0);
    sphere.set_theta_resolution(32);
    sphere.set_phi_resolution(16);
    sphere.set_center([1.0, 2.0, 3.0]);

    // `Update` is defined on `vtkAlgorithm` and reached through `Deref`.
    sphere.update();

    // `GetOutput` is defined on `vtkPolyDataAlgorithm`.
    let polydata = sphere.get_output().expect("the source produced output");

    // `GetPoints` is defined on `vtkPointSet`, `GetNumberOfPoints` on `vtkPoints`.
    let points = polydata.get_points().expect("polydata has points");
    let n = points.get_number_of_points();

    println!(
        "sphere: radius={}, theta={}, phi={}",
        sphere.get_radius(),
        sphere.get_theta_resolution(),
        sphere.get_phi_resolution(),
    );
    println!("generated {} points", n);

    assert!(n > 0, "expected a non-empty sphere");
    assert_eq!(sphere.get_radius(), 2.0);
}
