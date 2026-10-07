/* Bounded status queue and monitor replay in this standalone owned process. */
#include "sc_lobby_ui.c"
#include <assert.h>
#include <stdlib.h>
static DWORD wait_operation(HANDLE pipe,OVERLAPPED* operation,DWORD limit){
 DWORD wait=WaitForSingleObject(operation->hEvent,limit),done=0;
 if(wait!=WAIT_OBJECT_0){CancelIoEx(pipe,operation);WaitForSingleObject(operation->hEvent,1000);assert(0 && "owned pipe fixture timed out");}
 assert(GetOverlappedResult(pipe,operation,&done,FALSE));return done;
}
int main(void){
 assert(ScLobbyUiProtocolVersion()==3);
 InterlockedExchange(&initialization_phase,5);InterlockedExchange(&browser_count,7);
 for(int i=0;i<70;i++)status("JS_READY",7);
 status("private-lobby-secret-and-token",99);status(NULL,99);
 assert(own_counter(&js_ready_count)==70);assert(own_counter(&status_events_dropped)==8);
 unsigned ready=0;
 for(unsigned i=0;i<STATUS_QUEUE_CAPACITY;i++){
  StatusRecord* record=status_queue+i;
  if(own_counter(&record->state)!=2)continue;
  ready++;assert(record->length<STATUS_LINE_CAPACITY);
  char line[STATUS_LINE_CAPACITY];memcpy(line,record->line,record->length);line[record->length]=0;
  assert(strstr(line,"JS_READY value=7"));assert(!strstr(line,"private-lobby-secret"));
 }
 assert(ready==STATUS_QUEUE_CAPACITY);assert(ScLobbyUiReportStatus(NULL)==5);
 assert(own_counter(&js_ready_count)==70 && own_counter(&browser_count)==7);
 puts("PASS: bounded queue drops excess events, rejects unknown event text, preserves cumulative state and returns initialization summary");
 for(unsigned i=0;i<STATUS_QUEUE_CAPACITY;i++)InterlockedExchange(&status_queue[i].state,0);
 HANDLE worker=CreateThread(NULL,0,status_worker,NULL,0,NULL);assert(worker);CloseHandle(worker);
 Sleep(250); /* The first diagnostics intentionally have no monitor. */
 wchar_t pipe_name[MAX_PATH];_snwprintf(pipe_name,MAX_PATH,L"\\\\.\\pipe\\SCMultiTestLobby.Status.%lu",(unsigned long)GetCurrentProcessId());
 HANDLE server=CreateNamedPipeW(pipe_name,PIPE_ACCESS_INBOUND|FILE_FLAG_OVERLAPPED|FILE_FLAG_FIRST_PIPE_INSTANCE,PIPE_TYPE_BYTE|PIPE_READMODE_BYTE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,1,4096,4096,0,NULL);assert(server!=INVALID_HANDLE_VALUE);
 OVERLAPPED connect;memset(&connect,0,sizeof(connect));connect.hEvent=CreateEventW(NULL,TRUE,FALSE,NULL);assert(connect.hEvent);
 BOOL connected=ConnectNamedPipe(server,&connect);
 if(!connected){DWORD error=GetLastError();if(error==ERROR_PIPE_CONNECTED)SetEvent(connect.hEvent);else {assert(error==ERROR_IO_PENDING);wait_operation(server,&connect,7000);}}
 ULONG client=0;assert(GetNamedPipeClientProcessId(server,&client) && client==GetCurrentProcessId());
 char replay[STATUS_LINE_CAPACITY];memset(replay,0,sizeof(replay));
 OVERLAPPED read;memset(&read,0,sizeof(read));read.hEvent=CreateEventW(NULL,TRUE,FALSE,NULL);assert(read.hEvent);
 DWORD done=0;BOOL read_ok=ReadFile(server,replay,sizeof(replay)-1,&done,&read);
 if(!read_ok){assert(GetLastError()==ERROR_IO_PENDING);done=wait_operation(server,&read,1000);}
 assert(done>0 && done<sizeof(replay));replay[done]=0;
 assert(strstr(replay,"INITIALIZATION_STATE value=5"));assert(!strstr(replay,"private-lobby-secret"));
 CloseHandle(read.hEvent);CloseHandle(connect.hEvent);DisconnectNamedPipe(server);CloseHandle(server);
 puts("PASS: late local monitor receives retained initialization heartbeat after initial pipe records were lost; verified client is this fixture process");
 return 0;
}