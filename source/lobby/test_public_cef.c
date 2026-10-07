/* Standalone public-API contract and lifecycle fixtures. No game is connected. */
#define SC_LOBBY_TEST_FILE_SINK 1
#include "sc_lobby_ui.c"
#include <assert.h>
#include <stdlib.h>
static cef_task_t* queued;
static cef_string_visitor_t* pending_visitor;
static int64_t queued_delay;
static int executions,source_requests,reject_post,asynchronous_source;
static int browser_refs=1,frame_refs=1,runner_refs=1;
static int no_frame,frame_valid=1,fixture_ack=1;
static wchar_t lobby_url[]=L"https://localhost/webui/dist/GameLobbyPanel/GameLobbyPanel.html?guid=dont-log-me";
static wchar_t other_url[]=L"https://localhost/webui/dist/Other/Other.html?guid=dont-log-me";
static wchar_t* fixture_url=other_url;
static cef_frame_t fixture_frame;
static cef_browser_t fixture_browser;
static cef_task_runner_t fixture_runner;
static void CEF_CALLBACK fake_add(cef_base_t* b){if(b==&fixture_browser.base)browser_refs++;else if(b==&fixture_frame.base)frame_refs++;else runner_refs++;}
static int CEF_CALLBACK fake_release(cef_base_t* b){if(b==&fixture_browser.base)return --browser_refs==0;else if(b==&fixture_frame.base)return --frame_refs==0;else return --runner_refs==0;}
static int CEF_CALLBACK fake_one(cef_base_t* b){(void)b;return 1;}
static int CEF_CALLBACK fake_valid(cef_frame_t* f){(void)f;return frame_valid;}
static cef_frame_t* CEF_CALLBACK fake_frame(cef_browser_t* b){(void)b;if(no_frame)return NULL;fake_add(&fixture_frame.base);return &fixture_frame;}
static cef_string_t* CEF_CALLBACK fake_url(cef_frame_t* f){(void)f;cef_string_t* s=malloc(sizeof(*s));s->str=_wcsdup(fixture_url);s->length=wcslen(fixture_url);s->dtor=NULL;return s;}
static void __cdecl fake_free(cef_string_t* s){free(s->str);free(s);}
static void CEF_CALLBACK fake_eval(cef_frame_t* f,const cef_string_t* code,const cef_string_t* url,int line){
 assert(f==&fixture_frame);assert(code->str==bootstrap_code && code->length==bootstrap_length);assert(url->length==24);assert(line==1);
 cef_string_t current={fixture_url,wcslen(fixture_url),NULL};assert(is_lobby_url(&current));executions++;fixture_ack=1;
}
static void deliver_source(cef_string_visitor_t* visitor,int ack){
 static wchar_t ready[]=L"<html data-sc-lobby-ui=\"READY\"><div>private-lobby-name</div></html>";
 static wchar_t waiting[]=L"<html data-sc-lobby-ui=\"WAITING\"></html>";
 static wchar_t missing[]=L"<html><div>private-lobby-name</div></html>";
 wchar_t* html=ack==1?ready:ack==2?waiting:missing;cef_string_t source={html,wcslen(html),NULL};
 visitor->visit(visitor,&source);visitor->base.release(&visitor->base);
}
static void CEF_CALLBACK fake_source(cef_frame_t* f,cef_string_visitor_t* v){
 assert(f==&fixture_frame);source_requests++;
 /* The generated CToCpp wrapper first owns a reference, then consumes the
  * distinct transferred reference. Its remaining wrapper reference persists. */
 v->base.add_ref(&v->base);v->base.release(&v->base);
 if(asynchronous_source){assert(!pending_visitor);pending_visitor=v;}else deliver_source(v,fixture_ack);
}
static int CEF_CALLBACK fake_post(cef_task_runner_t* r,cef_task_t* t,int64_t delay){
 (void)r;assert(delay>=0 && delay<=5000);assert(!queued);
 t->base.add_ref(&t->base);t->base.release(&t->base); /* consume transfer */
 if(reject_post){t->base.release(&t->base);return 0;}queued=t;queued_delay=delay;return 1;
}
static cef_task_runner_t* __cdecl fake_runner(int thread){assert(thread==0);fake_add(&fixture_runner.base);return &fixture_runner;}
static cef_browser_t* __cdecl fake_create(const void* i,void* c,const cef_string_t* u,const void* s,void* r){(void)i;(void)c;(void)u;(void)s;(void)r;return &fixture_browser;}
static void tick(void){assert(queued);cef_task_t* current=queued;queued=NULL;current->execute(current);current->base.release(&current->base);}
static void flush_source(int ack){assert(pending_visitor);cef_string_visitor_t* v=pending_visitor;pending_visitor=NULL;deliver_source(v,ack);}
static void cancel_queue(void){assert(queued);cef_task_t* current=queued;queued=NULL;InterlockedExchange(&((BrowserTask*)current)->stopped,1);current->base.release(&current->base);}
static void assert_balanced(void){assert(browser_refs==1 && frame_refs==1 && runner_refs==1 && !queued && !pending_visitor);}
static void capture(void){cef_string_t url={fixture_url,wcslen(fixture_url),NULL};assert(create_hook(NULL,NULL,&url,NULL,NULL)==&fixture_browser);}
static int cef_fixture_versions[6]={3,1323,43,0,2357,0};
static int __cdecl official_fixture_version(int selector){assert(selector>=0 && selector<6);return cef_fixture_versions[selector];}
int main(void){
 assert(supported_cef_version(official_fixture_version));assert(!supported_cef_version(NULL));
 cef_fixture_versions[1]=2357;assert(!supported_cef_version(official_fixture_version));cef_fixture_versions[1]=1323;
 cef_fixture_versions[4]=9999;assert(!supported_cef_version(official_fixture_version));cef_fixture_versions[4]=2357;
 cef_fixture_versions[2]=99;assert(!supported_cef_version(official_fixture_version));cef_fixture_versions[2]=43;
 puts("PASS: official CEF version selectors pinned independently; incorrect commit/build mapping rejected");
 DeleteFileW(L"loader-selftest.log");wcscpy(status_path,L"loader-selftest.log");
 static wchar_t code[]=L"window.fixture=true;";bootstrap_code=code;bootstrap_length=wcslen(code);
 cef_string_t good={lobby_url,wcslen(lobby_url),NULL};assert(is_lobby_url(&good));
 cef_string_t bad={L"https://localhost/Other.html?redirect=/GameLobbyPanel/GameLobbyPanel.html",0,NULL};bad.length=wcslen(bad.str);assert(!is_lobby_url(&bad));
 bad.str=L"https://localhost/GameLobbyPanel/GameLobbyPanel.html/extra";bad.length=wcslen(bad.str);assert(!is_lobby_url(&bad));
 puts("PASS: exact lobby URL path guard rejects query redirection and foreign page paths");
 fixture_browser.base=(cef_base_t){sizeof(fixture_browser),fake_add,fake_release,fake_one};fixture_browser.get_main_frame=fake_frame;
 fixture_frame.base=(cef_base_t){sizeof(fixture_frame),fake_add,fake_release,fake_one};fixture_frame.is_valid=fake_valid;fixture_frame.get_url=fake_url;fixture_frame.execute_java_script=fake_eval;fixture_frame.get_source=fake_source;
 fixture_runner.base=(cef_base_t){sizeof(fixture_runner),fake_add,fake_release,fake_one};fixture_runner.post_delayed_task=fake_post;
 get_runner=fake_runner;free_user_string=fake_free;original_create=fake_create;
 capture();assert(browser_refs==2);
 for(int i=0;i<150;i++){tick();assert(queued && queued_delay==2000 && executions==0 && source_requests==0);}
 fixture_url=lobby_url;tick();assert(executions==1 && queued);tick();assert(own_counter(&((BrowserTask*)queued)->last_ack)==1 && queued_delay==5000);
 LONG ready_transitions=own_counter(&js_ready_count);
 for(int i=0;i<40;i++){tick();assert(queued && queued_delay==5000 && executions==1);}
 assert(own_counter(&js_ready_count)==ready_transitions);
 puts("PASS: valid non-lobby polling exceeds two minutes; READY remains monitored beyond20 acknowledgements without JS re-evaluation or duplicate READY logs");
 fixture_url=other_url;tick();assert(((BrowserTask*)queued)->phase==0 && queued_delay==2000 && executions==1);
 fixture_url=lobby_url;tick();assert(executions==2);tick();assert(queued_delay==5000);
 fixture_ack=0;tick();assert(own_counter(&((BrowserTask*)queued)->needs_bootstrap)==1 && executions==2);tick();assert(executions==3);tick();assert(queued_delay==5000);
 puts("PASS: same browser leaves/re-enters a room and same-URL fresh document triggers missing-marker reinjection only");
 asynchronous_source=1;int source_before=source_requests;tick();assert(pending_visitor && queued && ((BrowserTask*)queued)->refs==2 && own_counter(&((BrowserTask*)queued)->ack_inflight)==1);
 for(int i=0;i<4;i++){tick();}assert(source_requests==source_before+1);
 fixture_url=other_url;tick();assert(((BrowserTask*)queued)->phase==0);
 fixture_url=lobby_url;tick();assert(executions==4);
 flush_source(0);assert(own_counter(&((BrowserTask*)queued)->needs_bootstrap)==0 && own_counter(&((BrowserTask*)queued)->ack_inflight)==0);
 tick();assert(pending_visitor);flush_source(1);assert(((BrowserTask*)queued)->refs==1);
 asynchronous_source=0;
 puts("PASS: one async acknowledgement in flight retains task safely; stale previous-room acknowledgement cannot change current document state");
 no_frame=1;for(int i=0;i<10;i++)tick();assert(queued);
 no_frame=0;frame_valid=0;for(int i=0;i<5;i++)tick();assert(queued);
 frame_valid=1;tick();assert(((BrowserTask*)queued)->attempts==0 && executions==5);
 frame_valid=0;for(int i=0;i<60;i++)tick();assert_balanced();frame_valid=1;
 puts("PASS: temporary missing/invalid frames recover; consecutive invalid frame limit cleans task and browser references");
 fixture_url=lobby_url;reject_post=1;capture();assert_balanced();reject_post=0;
 capture();tick();asynchronous_source=1;reject_post=1;tick();assert(!queued && pending_visitor && browser_refs==2);
 assert(own_counter(&((StatusVisitor*)pending_visitor)->task->stopped)==1);flush_source(1);assert_balanced();reject_post=0;asynchronous_source=0;
 puts("PASS: initial post rejection and rejection with an outstanding source visitor release transferred/caller/browser ownership without cycles");
 capture();tick();asynchronous_source=1;tick();assert(pending_visitor && queued);cancel_queue();assert(browser_refs==2);flush_source(1);assert_balanced();asynchronous_source=0;
 puts("PASS: discarded scheduled task remains safe until final asynchronous visitor release, then all owned references balance");
 FILE* log=fopen("loader-selftest.log","rb");assert(log);char lines[32768];size_t n=fread(lines,1,sizeof(lines)-1,log);fclose(log);lines[n]=0;
 assert(strstr(lines,"JS_READY"));assert(strstr(lines,"FRAME_RETRY_PENDING"));assert(strstr(lines,"BROWSER_MONITOR_STOPPED"));assert(ScLobbyUiLastTaskStep(NULL)==9);
 assert(!strstr(lines,"TASK_ENTERED") && !strstr(lines,"MAIN_FRAME_QUERY_BEGIN") && !strstr(lines,"USER_STRING_FREE_END"));
 assert(!strstr(lines,"private-lobby-name") && !strstr(lines,"dont-log-me"));
 puts("PASS: only semantic state transitions are logged; fixed diagnostic last-step remains available without per-tick traces or private page data");
 return 0;
}