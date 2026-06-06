/* Minimal stub for libxdo.
 *
 * dioxus-desktop (via `muda`, `tray-icon`, `global-hotkey`) unconditionally links against
 * libxdo, even though this app uses no global hotkeys, tray icon, or menu accelerators (the
 * menubar is disabled with `Config::with_menu(None)`). The real libxdo is not installed in this
 * environment, so we provide these no-op symbols purely to satisfy the linker. None of them are
 * ever invoked at runtime by git-review.
 */
#include <stddef.h>

void *xdo_new(const char *display) { (void)display; return NULL; }
void xdo_free(void *xdo) { (void)xdo; }
int xdo_move_mouse(void *xdo, int x, int y, int screen) { (void)xdo; (void)x; (void)y; (void)screen; return 0; }
int xdo_move_mouse_relative(void *xdo, int dx, int dy) { (void)xdo; (void)dx; (void)dy; return 0; }
int xdo_mouse_down(void *xdo, unsigned long window, int button) { (void)xdo; (void)window; (void)button; return 0; }
int xdo_mouse_up(void *xdo, unsigned long window, int button) { (void)xdo; (void)window; (void)button; return 0; }
int xdo_click_window(void *xdo, unsigned long window, int button) { (void)xdo; (void)window; (void)button; return 0; }
int xdo_enter_text_window(void *xdo, unsigned long window, const char *string, unsigned int delay) { (void)xdo; (void)window; (void)string; (void)delay; return 0; }
int xdo_send_keysequence_window(void *xdo, unsigned long window, const char *keyseq, unsigned int delay) { (void)xdo; (void)window; (void)keyseq; (void)delay; return 0; }
int xdo_send_keysequence_window_down(void *xdo, unsigned long window, const char *keyseq, unsigned int delay) { (void)xdo; (void)window; (void)keyseq; (void)delay; return 0; }
int xdo_send_keysequence_window_up(void *xdo, unsigned long window, const char *keyseq, unsigned int delay) { (void)xdo; (void)window; (void)keyseq; (void)delay; return 0; }
