#define WIN32_LEAN_AND_MEAN
#include "cef2357_minimal.h"
#include "iat_data.h"
#include <wchar.h>
#include <stdio.h>
#include <string.h>

static HMODULE self_module;
static wchar_t status_path[MAX_PATH];
static wchar_t* bootstrap_code;
static size_t bootstrap_length;
static cef_create_browser_sync_fn original_create;
static cef_get_runner_fn get_runner;
static cef_userfree_free_fn free_user_string;
static LONG browser_count;

/* Status records belong to this module only. Producers never wait, open a pipe
 * or acquire a blocking lock. A full queue may drop a record; counters and the
 * latest initialization phase remain available for later diagnostic replay. */
#define STATUS_QUEUE_CAPACITY 64
#define STATUS_LINE_CAPACITY 256
static volatile LONG initialization_phase=0;
static volatile LONG last_task_step=0;
static volatile LONG status_worker_state=0;
static volatile LONG lobby_capture_count=0;
static volatile LONG bootstrap_queue_count=0;
static volatile LONG js_ready_count=0;
static volatile LONG js_waiting_count=0;
static volatile LONG js_error_count=0;
static volatile LONG status_events_dropped=0;
typedef struct {volatile LONG state;DWORD length;char line[STATUS_LINE_CAPACITY];} StatusRecord;
static StatusRecord status_queue[STATUS_QUEUE_CAPACITY];
static const char* const allowed_status_events[]={
 "MODULE_LOADED","BOOTSTRAP_VALIDATED","BOOTSTRAP_MISSING_OR_INVALID",
 "CEF_NOT_LOADED","CEF_VERSION_OR_API_UNSUPPORTED","CEF_2357_API_CONFIRMED","CEF_CHROMIUM_BUILD","CEF_COMMIT_NUMBER",
 "IAT_IMPORT_VALIDATION_FAILED","IAT_DATA_INTERCEPT_FAILED","IAT_WIN32_ERROR",
 "IAT_DATA_INTERCEPT_READY","TARGET_PROCESS_REJECTED","INIT_THREAD_FAILED",
 "JS_READY","JS_WAITING","JS_ERROR","JS_STATUS_OTHER","JS_ACK_NOT_PRESENT",
 "UI_RUNNER_UNAVAILABLE","UI_TASK_REJECTED","BROWSER_ABI_UNSUPPORTED",
 "LOBBY_FRAME_NOT_FOUND","FRAME_INVALID_OR_ABI_UNSUPPORTED","LOBBY_FRAME_LEFT",
 "NON_LOBBY_BROWSER_TIMEOUT","JS_API_UNAVAILABLE","BOOTSTRAP_QUEUED",
 "ACK_MONITOR_FINISHED","CREATE_BROWSER_RETURNED_NULL","LOBBY_BROWSER_CAPTURED",
 "BROWSER_CAPTURED","CAPTURE_ABI_UNSUPPORTED","TASK_ALLOCATION_FAILED",
 "UI_TASK_SCHEDULED","INITIALIZATION_STATE","STATUS_THREAD_STATE",
 "TOTAL_BROWSER_CAPTURES","TOTAL_LOBBY_CAPTURES","TOTAL_BOOTSTRAP_QUEUED",
 "TOTAL_JS_READY","TOTAL_JS_WAITING","TOTAL_JS_ERROR","STATUS_EVENTS_DROPPED",
 "TASK_ENTERED","MAIN_FRAME_QUERY_BEGIN","MAIN_FRAME_QUERY_END",
 "FRAME_VALIDITY_BEGIN","FRAME_VALIDITY_END","FRAME_URL_QUERY_BEGIN",
 "FRAME_URL_QUERY_END","USER_STRING_FREE_BEGIN","USER_STRING_FREE_END","LAST_TASK_STEP","FRAME_RETRY_PENDING","BROWSER_MONITOR_STOPPED"
};
static LONG own_counter(volatile LONG* value){return InterlockedCompareExchange(value,0,0);}
static const char* known_status_event(const char* event){
 if(!event)return NULL;
 for(size_t i=0;i<sizeof(allowed_status_events)/sizeof(allowed_status_events[0]);i++)
  if(strcmp(event,allowed_status_events[i])==0)return allowed_status_events[i];
 return NULL;
}
static void remember_status(const char* event){
 if(strcmp(event,"MODULE_LOADED")==0)InterlockedExchange(&initialization_phase,2);
 else if(strcmp(event,"BOOTSTRAP_VALIDATED")==0)InterlockedExchange(&initialization_phase,3);
 else if(strcmp(event,"CEF_2357_API_CONFIRMED")==0)InterlockedExchange(&initialization_phase,4);
 else if(strcmp(event,"IAT_DATA_INTERCEPT_READY")==0)InterlockedExchange(&initialization_phase,5);
 else if(strcmp(event,"TARGET_PROCESS_REJECTED")==0)InterlockedExchange(&initialization_phase,101);
 else if(strcmp(event,"BOOTSTRAP_MISSING_OR_INVALID")==0)InterlockedExchange(&initialization_phase,103);
 else if(strcmp(event,"CEF_NOT_LOADED")==0)InterlockedExchange(&initialization_phase,104);
 else if(strcmp(event,"CEF_VERSION_OR_API_UNSUPPORTED")==0)InterlockedExchange(&initialization_phase,105);
 else if(strcmp(event,"IAT_IMPORT_VALIDATION_FAILED")==0)InterlockedExchange(&initialization_phase,106);
 else if(strcmp(event,"IAT_DATA_INTERCEPT_FAILED")==0)InterlockedExchange(&initialization_phase,107);
 else if(strcmp(event,"INIT_THREAD_FAILED")==0)InterlockedExchange(&initialization_phase,108);
 else if(strcmp(event,"LOBBY_BROWSER_CAPTURED")==0)InterlockedIncrement(&lobby_capture_count);
 else if(strcmp(event,"BOOTSTRAP_QUEUED")==0)InterlockedIncrement(&bootstrap_queue_count);
 else if(strcmp(event,"JS_READY")==0)InterlockedIncrement(&js_ready_count);
 else if(strcmp(event,"JS_WAITING")==0)InterlockedIncrement(&js_waiting_count);
 else if(strcmp(event,"JS_ERROR")==0)InterlockedIncrement(&js_error_count);
}
static void status(const char* event,int number){
 const char* fixed=known_status_event(event);
 if(!fixed){InterlockedIncrement(&status_events_dropped);return;}
 remember_status(fixed);
 SYSTEMTIME t;GetLocalTime(&t);
 char line[STATUS_LINE_CAPACITY];int length=_snprintf(line,sizeof(line),"%02u:%02u:%02u pid=%lu %s value=%d\r\n",t.wHour,t.wMinute,t.wSecond,(unsigned long)GetCurrentProcessId(),fixed,number);
 if(length<=0 || (size_t)length>=sizeof(line)){InterlockedIncrement(&status_events_dropped);return;}
#ifdef SC_LOBBY_TEST_FILE_SINK
 /* Synchronous files are used only by the independent CEF reference fixture. */
 HANDLE file=CreateFileW(status_path,FILE_APPEND_DATA,FILE_SHARE_READ|FILE_SHARE_WRITE,NULL,OPEN_ALWAYS,FILE_ATTRIBUTE_NORMAL,NULL);
 if(file!=INVALID_HANDLE_VALUE){DWORD done=0;WriteFile(file,line,(DWORD)length,&done,NULL);CloseHandle(file);}
#else
 for(unsigned i=0;i<STATUS_QUEUE_CAPACITY;i++){
  StatusRecord* record=status_queue+i;
  if(InterlockedCompareExchange(&record->state,1,0)!=0)continue;
  memcpy(record->line,line,(size_t)length);record->length=(DWORD)length;
  InterlockedExchange(&record->state,2);return;
 }
 InterlockedIncrement(&status_events_dropped);
#endif
}
/* This owned export reads only this DLL's state and queues fixed diagnostic
 * snapshots. It has the public Windows thread-entry ABI and accepts no input. */
__declspec(dllexport) DWORD WINAPI ScLobbyUiReportStatus(void* unused){
 (void)unused;LONG phase=own_counter(&initialization_phase);
 status("INITIALIZATION_STATE",phase);
 status("LAST_TASK_STEP",own_counter(&last_task_step));
 status("STATUS_THREAD_STATE",own_counter(&status_worker_state));
 status("TOTAL_BROWSER_CAPTURES",InterlockedCompareExchange(&browser_count,0,0));
 status("TOTAL_LOBBY_CAPTURES",own_counter(&lobby_capture_count));
 status("TOTAL_BOOTSTRAP_QUEUED",own_counter(&bootstrap_queue_count));
 status("TOTAL_JS_READY",own_counter(&js_ready_count));
 status("TOTAL_JS_WAITING",own_counter(&js_waiting_count));
 status("TOTAL_JS_ERROR",own_counter(&js_error_count));
 status("STATUS_EVENTS_DROPPED",own_counter(&status_events_dropped));
 return (DWORD)phase;
}
/* Fixed marker IDs describe calls in our own callback; no pointer or page data. */
static void task_step(LONG step,const char* event,int browser_id){
 InterlockedExchange(&last_task_step,step);
#ifdef SC_LOBBY_TRACE_TASK_STEPS
 status(event,browser_id);
#else
 (void)event;(void)browser_id;
#endif
}
__declspec(dllexport) DWORD WINAPI ScLobbyUiLastTaskStep(void* unused){
 (void)unused;return (DWORD)own_counter(&last_task_step);
}
static int send_status_record(const StatusRecord* record){
 /* Retry work is confined to this module's own background thread. Each pipe
  * connection has at most two attempts and a busy-server wait of <=100ms. */
 for(unsigned attempt=0;attempt<2;attempt++){
  HANDLE pipe=CreateFileW(status_path,GENERIC_WRITE,0,NULL,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,NULL);
  if(pipe==INVALID_HANDLE_VALUE){
   if(GetLastError()==ERROR_PIPE_BUSY){WaitNamedPipeW(status_path,100);continue;}
   return 0;
  }
  DWORD done=0;BOOL ok=WriteFile(pipe,record->line,record->length,&done,NULL);CloseHandle(pipe);
  if(ok && done==record->length)return 1;
 }
 return 0;
}
static DWORD WINAPI status_worker(void* unused){
 (void)unused;
 int path_length=_snwprintf(status_path,MAX_PATH,L"\\\\.\\pipe\\SCMultiTestLobby.Status.%lu",(unsigned long)GetCurrentProcessId());
 if(path_length<=0 || path_length>=MAX_PATH){InterlockedExchange(&status_worker_state,2);return 1;}
 status_path[path_length]=0;InterlockedExchange(&status_worker_state,1);
 ULONGLONG next_heartbeat=0;
 for(;;){
  /* Small batches keep a missing monitor from holding diagnostic replay up. */
  unsigned drained=0;
  for(unsigned i=0;i<STATUS_QUEUE_CAPACITY && drained<8;i++){
   StatusRecord* record=status_queue+i;
   if(InterlockedCompareExchange(&record->state,3,2)!=2)continue;
   if(!send_status_record(record))InterlockedIncrement(&status_events_dropped);
   InterlockedExchange(&record->state,0);drained++;
  }
  ULONGLONG now=GetTickCount64();
  if(now>=next_heartbeat){ScLobbyUiReportStatus(NULL);next_heartbeat=now+4000;}
  Sleep(50);
 }
}
static int is_lobby_url(const cef_string_t* url) {
 static const wchar_t wanted[]=L"/GameLobbyPanel/GameLobbyPanel.html";
 if(!url || !url->str || url->length>16384)return 0;
 size_t limit=0;
 while(limit<url->length && url->str[limit]!=L'?' && url->str[limit]!=L'#')limit++;
 size_t n=(sizeof(wanted)/sizeof(wanted[0]))-1;
 return limit>=n && _wcsnicmp(url->str+limit-n,wanted,n)==0;
}

static int read_bootstrap(void) {
 wchar_t path[MAX_PATH]; DWORD count=GetModuleFileNameW(self_module,path,MAX_PATH);
 if(!count || count>=MAX_PATH)return 0;
 wchar_t* slash=wcsrchr(path,L'\\'); if(!slash)return 0;
 *++slash=0;
 if(wcslen(path)+14>=MAX_PATH)return 0;
 wcscat(path,L"bootstrap.js");
 HANDLE file=CreateFileW(path,GENERIC_READ,FILE_SHARE_READ,NULL,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,NULL);
 if(file==INVALID_HANDLE_VALUE)return 0;
 LARGE_INTEGER length;
 if(!GetFileSizeEx(file,&length) || length.QuadPart<16 || length.QuadPart>131072){CloseHandle(file);return 0;}
 char* utf8=(char*)HeapAlloc(GetProcessHeap(),0,(SIZE_T)length.QuadPart+1);
 if(!utf8){CloseHandle(file);return 0;}
 DWORD done=0;BOOL ok=ReadFile(file,utf8,(DWORD)length.QuadPart,&done,NULL);CloseHandle(file);
 if(!ok || done!=(DWORD)length.QuadPart){HeapFree(GetProcessHeap(),0,utf8);return 0;}
 utf8[done]=0;
 int chars=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,utf8,(int)done,NULL,0);
 if(chars<=0){HeapFree(GetProcessHeap(),0,utf8);return 0;}
 bootstrap_code=(wchar_t*)HeapAlloc(GetProcessHeap(),0,((size_t)chars+1)*sizeof(wchar_t));
 if(!bootstrap_code){HeapFree(GetProcessHeap(),0,utf8);return 0;}
 MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,utf8,(int)done,bootstrap_code,chars);
 bootstrap_code[chars]=0;bootstrap_length=(size_t)chars;HeapFree(GetProcessHeap(),0,utf8);return 1;
}

typedef struct {
 cef_task_t api;LONG refs;cef_browser_t* browser;int id;
 unsigned attempts;unsigned phase;volatile LONG generation;
 volatile LONG ack_inflight;volatile LONG needs_bootstrap;
 volatile LONG last_ack;volatile LONG stopped;
} BrowserTask;
typedef struct {cef_string_visitor_t api;LONG refs;BrowserTask* task;LONG generation;} StatusVisitor;
static void CEF_CALLBACK visitor_add_ref(cef_base_t* base){InterlockedIncrement(&((StatusVisitor*)base)->refs);}
static int CEF_CALLBACK visitor_release(cef_base_t* base){
 StatusVisitor* visitor=(StatusVisitor*)base;
 if(InterlockedDecrement(&visitor->refs)==0){
  BrowserTask* task=visitor->task;
  if(task){InterlockedExchange(&task->ack_inflight,0);task->api.base.release(&task->api.base);}
  HeapFree(GetProcessHeap(),0,visitor);return 1;
 }
 return 0;
}
static int CEF_CALLBACK visitor_has_one_ref(cef_base_t* base){return InterlockedCompareExchange(&((StatusVisitor*)base)->refs,0,0)==1;}
static void publish_ack(StatusVisitor* visitor,LONG ack,const char* event){
 BrowserTask* task=visitor->task;
 /* A delayed previous-document acknowledgement cannot change the new lobby's
  * state. The visitor owns one task reference, without a task->visitor cycle. */
 if(!task || own_counter(&task->stopped) || !task->phase || visitor->generation!=own_counter(&task->generation))return;
 if(ack==5)InterlockedExchange(&task->needs_bootstrap,1);
 else InterlockedExchange(&task->needs_bootstrap,0);
 if(InterlockedExchange(&task->last_ack,ack)!=ack)status(event,task->id);
}
static void CEF_CALLBACK visit_status(cef_string_visitor_t* visitor,const cef_string_t* source){
 /* Inspect only our fixed acknowledgement attribute, never log source, names,
  * URLs, tokens or page text. READY is logged only when its state changes. */
 static const wchar_t key[]=L"data-sc-lobby-ui=\"";
 const size_t keylen=(sizeof(key)/sizeof(key[0]))-1;
 StatusVisitor* own=(StatusVisitor*)visitor;
 if(!source || !source->str || source->length>1048576)return;
 for(size_t i=0;i+keylen+5<source->length;i++){
  if(wmemcmp(source->str+i,key,keylen)!=0)continue;
  const wchar_t* value=source->str+i+keylen;size_t left=source->length-i-keylen;
  if(left>=6 && wmemcmp(value,L"READY\"",6)==0)publish_ack(own,1,"JS_READY");
  else if(left>=8 && wmemcmp(value,L"WAITING\"",8)==0)publish_ack(own,2,"JS_WAITING");
  else if(left>=5 && wmemcmp(value,L"ERROR",5)==0)publish_ack(own,3,"JS_ERROR");
  else publish_ack(own,4,"JS_STATUS_OTHER");
  return;
 }
 publish_ack(own,5,"JS_ACK_NOT_PRESENT");
}
static void request_ack(cef_frame_t* frame,BrowserTask* task){
 if(!CEF_HAS(frame,get_source) || own_counter(&task->stopped) || InterlockedCompareExchange(&task->ack_inflight,1,0)!=0)return;
 StatusVisitor* visitor=(StatusVisitor*)HeapAlloc(GetProcessHeap(),HEAP_ZERO_MEMORY,sizeof(*visitor));
 if(!visitor){InterlockedExchange(&task->ack_inflight,0);return;}
 visitor->api.base.size=sizeof(cef_string_visitor_t);visitor->refs=1;visitor->task=task;visitor->generation=own_counter(&task->generation);
 visitor->api.base.add_ref=visitor_add_ref;visitor->api.base.release=visitor_release;
 visitor->api.base.has_one_ref=visitor_has_one_ref;visitor->api.visit=visit_status;
 task->api.base.add_ref(&task->api.base);
 /* CEF consumes a transferred CAPI reference. Keep our caller reference until
  * get_source returns; the final visitor release also releases the task. */
 visitor->api.base.add_ref(&visitor->api.base);
 frame->get_source(frame,&visitor->api);visitor->api.base.release(&visitor->api.base);
}
static void CEF_CALLBACK task_add_ref(cef_base_t* base){InterlockedIncrement(&((BrowserTask*)base)->refs);}
static int CEF_CALLBACK task_release(cef_base_t* base){
 BrowserTask* task=(BrowserTask*)base;
 if(InterlockedDecrement(&task->refs)==0){
  if(task->browser && task->browser->base.release)task->browser->base.release(&task->browser->base);
  HeapFree(GetProcessHeap(),0,task);return 1;
 }
 return 0;
}
static int CEF_CALLBACK task_has_one_ref(cef_base_t* base){return InterlockedCompareExchange(&((BrowserTask*)base)->refs,0,0)==1;}
static int schedule_task(BrowserTask* task,int64_t delay){
 if(own_counter(&task->stopped))return 0;
 cef_task_runner_t* runner=get_runner(0); /* public TID_UI */
 if(!runner){status("UI_RUNNER_UNAVAILABLE",task->id);InterlockedExchange(&task->stopped,1);return 0;}
 int result=0;
 if(CEF_HAS(runner,post_delayed_task)){
  /* CToCpp::Wrap consumes this distinct transferred reference. */
  task->api.base.add_ref(&task->api.base);
  result=runner->post_delayed_task(runner,&task->api,delay);
 }
 if(runner->base.release)runner->base.release(&runner->base);
 if(!result){status("UI_TASK_REJECTED",task->id);InterlockedExchange(&task->stopped,1);}
 return result;
}
static void leave_lobby(BrowserTask* task){
 if(task->phase){
  status("LOBBY_FRAME_LEFT",task->id);task->phase=0;
  InterlockedIncrement(&task->generation);InterlockedExchange(&task->last_ack,0);
  InterlockedExchange(&task->needs_bootstrap,0);
 }
}
static void stop_task(BrowserTask* task,const char* event){
 InterlockedExchange(&task->stopped,1);InterlockedIncrement(&task->generation);status(event,task->id);
}
static void execute_invalid_retry(BrowserTask* task){
 leave_lobby(task);
 if(++task->attempts==1)status("FRAME_RETRY_PENDING",task->id);
 if(task->attempts<60)schedule_task(task,2000);else stop_task(task,"BROWSER_MONITOR_STOPPED");
}
static void CEF_CALLBACK execute_task(cef_task_t* api){
 BrowserTask* task=(BrowserTask*)api;task_step(1,"TASK_ENTERED",task->id);
 if(own_counter(&task->stopped))return;
 if(!CEF_HAS(task->browser,get_main_frame)){stop_task(task,"BROWSER_ABI_UNSUPPORTED");return;}
 task_step(2,"MAIN_FRAME_QUERY_BEGIN",task->id);
 cef_frame_t* frame=task->browser->get_main_frame(task->browser);
 task_step(3,"MAIN_FRAME_QUERY_END",task->id);
 if(!frame){execute_invalid_retry(task);return;}
 if(!CEF_HAS(frame,get_url) || !CEF_HAS(frame,is_valid)){
  stop_task(task,"FRAME_INVALID_OR_ABI_UNSUPPORTED");if(frame->base.release)frame->base.release(&frame->base);return;
 }
 task_step(4,"FRAME_VALIDITY_BEGIN",task->id);int frame_valid=frame->is_valid(frame);
 task_step(5,"FRAME_VALIDITY_END",task->id);
 if(!frame_valid){execute_invalid_retry(task);frame->base.release(&frame->base);return;}
 task->attempts=0;
 task_step(6,"FRAME_URL_QUERY_BEGIN",task->id);cef_string_t* url=frame->get_url(frame);
 task_step(7,"FRAME_URL_QUERY_END",task->id);int lobby=is_lobby_url(url);
 if(url){task_step(8,"USER_STRING_FREE_BEGIN",task->id);free_user_string(url);task_step(9,"USER_STRING_FREE_END",task->id);}
 if(!lobby){
  leave_lobby(task);schedule_task(task,2000);frame->base.release(&frame->base);return;
 }
 if(!task->phase || own_counter(&task->needs_bootstrap)){
  if(!CEF_HAS(frame,execute_java_script)){stop_task(task,"JS_API_UNAVAILABLE");frame->base.release(&frame->base);return;}
  InterlockedIncrement(&task->generation);InterlockedExchange(&task->needs_bootstrap,0);
  InterlockedExchange(&task->last_ack,0);task->phase=1;
  static wchar_t label[]=L"sc-lobby-ui-bootstrap.js";
  cef_string_t code={bootstrap_code,bootstrap_length,NULL};cef_string_t source_url={label,(sizeof(label)/sizeof(label[0]))-1,NULL};
  frame->execute_java_script(frame,&code,&source_url,1);status("BOOTSTRAP_QUEUED",task->id);
  schedule_task(task,2000);
 }else{
  request_ack(frame,task);
  /* READY is stable: no execute_java_script or forceUpdate is repeated. The
   * source visitor detects same-URL document reload via missing own marker. */
  schedule_task(task,own_counter(&task->last_ack)==1?5000:2000);
 }
 frame->base.release(&frame->base);
}
static cef_browser_t* __cdecl create_hook(const void* info,void* client,const cef_string_t* url,const void* settings,void* request_context){
 cef_browser_t* browser=original_create(info,client,url,settings,request_context);
 if(!browser){status("CREATE_BROWSER_RETURNED_NULL",0);return browser;}
 int id=(int)InterlockedIncrement(&browser_count);
 status(is_lobby_url(url)?"LOBBY_BROWSER_CAPTURED":"BROWSER_CAPTURED",id);
 if(!CEF_HAS(browser,get_main_frame) || !browser->base.add_ref || !browser->base.release){status("CAPTURE_ABI_UNSUPPORTED",id);return browser;}
 BrowserTask* task=(BrowserTask*)HeapAlloc(GetProcessHeap(),HEAP_ZERO_MEMORY,sizeof(*task));
 if(!task){status("TASK_ALLOCATION_FAILED",id);return browser;}
 task->api.base.size=sizeof(cef_task_t);task->refs=1;task->browser=browser;task->id=id;
 task->api.base.add_ref=task_add_ref;task->api.base.release=task_release;task->api.base.has_one_ref=task_has_one_ref;task->api.execute=execute_task;
 browser->base.add_ref(&browser->base);
 if(schedule_task(task,250))status("UI_TASK_SCHEDULED",id);
 task->api.base.release(&task->api.base);
 return browser;
}

/* Official branch2357 public generator documents selectors:
 * 0 CEF_VERSION_MAJOR, 1 CEF_COMMIT_NUMBER, 2 CHROME_VERSION_MAJOR,
 * 3 CHROME_VERSION_MINOR, 4 CHROME_VERSION_BUILD, 5 CHROME_VERSION_PATCH.
 * https://raw.githubusercontent.com/chromiumembedded/cef/2357/tools/make_version_header.py
 * The target libcef file hash is independently pinned by the launcher. */
static int supported_cef_version(cef_version_info_fn version){
 return version && version(0)==3 && version(1)==1323 && version(2)==43 && version(4)==2357;
}
static DWORD WINAPI initialise(void* unused){
 (void)unused;
 wchar_t exe[MAX_PATH];DWORD n=GetModuleFileNameW(NULL,exe,MAX_PATH);
 if(!n || n>=MAX_PATH){status("TARGET_PROCESS_REJECTED",1);return 1;}
 wchar_t* basename=wcsrchr(exe,L'\\');basename=basename?basename+1:exe;
 if(_wcsicmp(basename,L"SceneCefBrowser.exe")!=0){status("TARGET_PROCESS_REJECTED",2);return 2;}
 status("MODULE_LOADED",0);
 if(!read_bootstrap()){status("BOOTSTRAP_MISSING_OR_INVALID",GetLastError());return 5;}
 status("BOOTSTRAP_VALIDATED",0);
 HMODULE cef=NULL;
 for(unsigned i=0;i<200 && !cef;i++){cef=GetModuleHandleW(L"libcef.dll");if(!cef)Sleep(100);}
 if(!cef){status("CEF_NOT_LOADED",0);return 6;}
 cef_version_info_fn version=(cef_version_info_fn)(void*)GetProcAddress(cef,"cef_version_info");
 get_runner=(cef_get_runner_fn)(void*)GetProcAddress(cef,"cef_task_runner_get_for_thread");
 free_user_string=(cef_userfree_free_fn)(void*)GetProcAddress(cef,"cef_string_userfree_utf16_free");
 void* create=(void*)GetProcAddress(cef,"cef_browser_host_create_browser_sync");
 if(version){status("CEF_CHROMIUM_BUILD",version(4));status("CEF_COMMIT_NUMBER",version(1));}
 if(!supported_cef_version(version) || !get_runner || !free_user_string || !create){status("CEF_VERSION_OR_API_UNSUPPORTED",version?version(1):-1);return 7;}
 status("CEF_2357_API_CONFIRMED",0);
 BYTE* image=(BYTE*)GetModuleHandleW(NULL);
 SIZE_T image_size=iat_own_image_size(image);void** slot=NULL;
 enum iat_result result=iat_find_cef_slot(image,image_size,&slot);
 if(result!=IAT_OK){status("IAT_IMPORT_VALIDATION_FAILED",result);return 8;}
 /* Publish the original public function before the atomic data-pointer switch. */
 original_create=(cef_create_browser_sync_fn)create;
 DWORD win32=0;result=iat_replace_data_slot(slot,create,(void*)create_hook,&win32);
 if(result!=IAT_OK){status("IAT_DATA_INTERCEPT_FAILED",result);if(win32)status("IAT_WIN32_ERROR",(int)win32);return 9;}
 status("IAT_DATA_INTERCEPT_READY",0);return 0;
}
__declspec(dllexport) int __cdecl ScLobbyUiProtocolVersion(void){return 3;}
BOOL WINAPI DllMain(HINSTANCE module,DWORD reason,LPVOID reserved){
 (void)reserved;
 if(reason==DLL_PROCESS_ATTACH){
  self_module=module;DisableThreadLibraryCalls(module);InterlockedExchange(&initialization_phase,1);
  HANDLE reporter=CreateThread(NULL,0,status_worker,NULL,0,NULL);
  if(reporter)CloseHandle(reporter);else InterlockedExchange(&status_worker_state,2);
  HANDLE worker=CreateThread(NULL,0,initialise,NULL,0,NULL);
  if(worker)CloseHandle(worker);else status("INIT_THREAD_FAILED",(int)GetLastError());
 }
 /* The module remains loaded for the helper process lifetime. No code or CEF
  * references are freed under loader lock; game/helper exit removes the data import interception. */
 return TRUE;
}