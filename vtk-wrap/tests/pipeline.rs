//! End-to-end tests for the generated bindings.

use vtk_wrap::vtkSphereSource;

#[test]
fn sphere_source_pipeline() {
    let sphere = vtkSphereSource::new();
    sphere.set_radius(2.0);
    sphere.set_theta_resolution(16);
    sphere.set_phi_resolution(8);
    sphere.set_center([1.0, -2.0, 0.5]);

    // `Update` comes from `vtkAlgorithm`.
    sphere.update();

    // `GetOutput` comes from `vtkPolyDataAlgorithm`, `GetPoints` from
    // `vtkPointSet`, `GetNumberOfPoints` from `vtkPoints`; all reached through
    // `Deref`.
    let polydata = sphere.get_output().expect("output");
    let points = polydata.get_points().expect("points");
    let n = points.get_number_of_points();
    assert!(n > 0, "sphere produced no points");

    // `GetClassName` comes from `vtkObjectBase`.
    assert_eq!(sphere.get_class_name(), "vtkSphereSource");
    assert_eq!(sphere.get_radius(), 2.0);
    assert_eq!(sphere.get_theta_resolution(), 16);
}

#[test]
fn reference_counting() {
    let sphere = vtkSphereSource::new();
    let before = sphere.get_reference_count();
    {
        let clone = sphere.clone();
        assert_eq!(clone.get_reference_count(), before + 1);
    }
    assert_eq!(sphere.get_reference_count(), before);
}
