/* Minimal ABI prefixes derived from CEF branch2357 public CAPI headers.
 * Original headers/licence are preserved in ../../cef2357-reference.
 * This module uses only libcef exports/public object methods, no game offsets. */
#ifndef SC_CEF2357_MINIMAL_H
#define SC_CEF2357_MINIMAL_H
#include <windows.h>
#include <stddef.h>
#include <stdint.h>
#define CEF_CALLBACK __stdcall
#define CEF_HAS(s,m) ((s) && (s)->base.size >= offsetof(__typeof__(*(s)),m)+sizeof((s)->m) && (s)->m)
typedef struct cef_base_s cef_base_t;
struct cef_base_s {size_t size; void (CEF_CALLBACK *add_ref)(cef_base_t*); int (CEF_CALLBACK *release)(cef_base_t*); int (CEF_CALLBACK *has_one_ref)(cef_base_t*);};
typedef struct {wchar_t* str; size_t length; void (*dtor)(wchar_t*);} cef_string_t;
typedef struct cef_browser_s cef_browser_t;
typedef struct cef_frame_s cef_frame_t;
typedef struct cef_visitor_s cef_string_visitor_t;
typedef struct cef_task_s cef_task_t;
typedef struct cef_runner_s cef_task_runner_t;
struct cef_browser_s {
 cef_base_t base;
 void* get_host; void* can_go_back; void* go_back; void* can_go_forward; void* go_forward;
 void* is_loading; void* reload; void* reload_ignore_cache; void* stop_load;
 int (CEF_CALLBACK *get_identifier)(cef_browser_t*);
 void* is_same; void* is_popup; void* has_document;
 cef_frame_t* (CEF_CALLBACK *get_main_frame)(cef_browser_t*);
};
struct cef_frame_s {
 cef_base_t base;
 int (CEF_CALLBACK *is_valid)(cef_frame_t*);
 void* undo; void* redo; void* cut; void* copy; void* paste; void* del; void* select_all; void* view_source;
 void (CEF_CALLBACK *get_source)(cef_frame_t*,cef_string_visitor_t*);
 void* get_text; void* load_request; void* load_url; void* load_string;
 void (CEF_CALLBACK *execute_java_script)(cef_frame_t*,const cef_string_t*,const cef_string_t*,int);
 void* is_main; void* is_focused; void* get_name; void* get_identifier; void* get_parent;
 cef_string_t* (CEF_CALLBACK *get_url)(cef_frame_t*);
};
struct cef_visitor_s {cef_base_t base; void (CEF_CALLBACK *visit)(cef_string_visitor_t*,const cef_string_t*);};
struct cef_task_s {cef_base_t base; void (CEF_CALLBACK *execute)(cef_task_t*);};
struct cef_runner_s {
 cef_base_t base; void* is_same; void* belongs_to_current_thread; void* belongs_to_thread;
 int (CEF_CALLBACK *post_task)(cef_task_runner_t*,cef_task_t*);
 int (CEF_CALLBACK *post_delayed_task)(cef_task_runner_t*,cef_task_t*,int64_t);
};
typedef cef_browser_t* (__cdecl *cef_create_browser_sync_fn)(const void*,void*,const cef_string_t*,const void*,void*);
typedef cef_task_runner_t* (__cdecl *cef_get_runner_fn)(int);
typedef void (__cdecl *cef_userfree_free_fn)(cef_string_t*);
typedef int (__cdecl *cef_version_info_fn)(int);
_Static_assert(sizeof(cef_base_t)==32,"x64 CEF2357 base ABI");
_Static_assert(offsetof(cef_browser_t,get_main_frame)==0x88,"CEF2357 browser ABI");
_Static_assert(offsetof(cef_frame_t,execute_java_script)==0x90,"CEF2357 frame ABI");
_Static_assert(offsetof(cef_frame_t,get_url)==0xc0,"CEF2357 frame URL ABI");
_Static_assert(offsetof(cef_task_runner_t,post_delayed_task)==0x40,"CEF2357 taskrunner ABI");
#endif