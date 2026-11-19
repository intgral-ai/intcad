# Linux VM run (Ubuntu 24.04, XFCE/X11, llvmpipe)

`intcad-e2e --task cef-theme accept`, commit `219916a85`, artifact
`f25bb0acfb586eab4aa0f1fe77842f4a9199d7859c280d54935a698fd66694a8`.

What it shows:

- Steps 1-5 **PASS**: the CEF build produced an artifact in the Ubuntu 24.04 container, it was
  deployed to the guest, the app opened the sample plate, a real drag rotated it and a face click
  selected it.
- Steps 6-8 **BLOCKED**, not failed: the guest renders with Vulkan llvmpipe, which has no
  `VK_EXT_external_memory_dma_buf`, so the app cannot import the page texture. The panel shows the
  intended diagnostic ("The app could not start its page renderer…") and the CAD view keeps
  working (`no-interop.png`). That is the no-GPU-interop gate, not a visual test.
- CEF was installed into the guest at `~/intcad-e2e/cef` and passed through `CEF_PATH` /
  `LD_LIBRARY_PATH` by the local driver patch; the CEF runtime is present in the build (the binary
  links `libcef.so`).

Theme/visual acceptance therefore runs on the host, where a GPU can export DMA-BUFs.
