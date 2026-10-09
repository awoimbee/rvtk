// Support routine for the link probe.
//
// The generated `CMakeLists.txt` builds `vtk_wrap_link_probe`, a throw-away
// executable that links the static shim.  It exists so that
// `vtk-wrap-sys/build.rs` can read CMake's own link line (and so that the static
// VTK archives are proven to link on this platform); it is never installed.
//
// The call to `vtk_wrap_register` makes the linker pull the shim in, which in turn
// pulls in the static VTK archives.

extern "C" void vtk_wrap_register(void* obj);

int main()
{
  vtk_wrap_register(nullptr);
  return 0;
}
