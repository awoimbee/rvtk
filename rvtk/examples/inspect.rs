use vtk::*;
fn show(name: &str, cn: String, is_a_self: i32, is_a_base: i32) {
    println!("{name}: class_name={cn:?} is_a(self)={is_a_self} is_a(vtkObjectBase)={is_a_base}");
}
fn main() {
    {
        let a = vtkDoubleArray::new();
        show(
            "vtkDoubleArray",
            a.get_class_name(),
            a.is_a("vtkDoubleArray"),
            a.is_a("vtkObjectBase"),
        );
    }
    {
        let a = vtkIntArray::new();
        show(
            "vtkIntArray",
            a.get_class_name(),
            a.is_a("vtkIntArray"),
            a.is_a("vtkObjectBase"),
        );
    }
    {
        let a = vtkFloatArray::new();
        show(
            "vtkFloatArray",
            a.get_class_name(),
            a.is_a("vtkFloatArray"),
            a.is_a("vtkObjectBase"),
        );
    }
    {
        let a = vtkURI::new();
        show(
            "vtkURI",
            a.get_class_name(),
            a.is_a("vtkURI"),
            a.is_a("vtkObjectBase"),
        );
    }
}
