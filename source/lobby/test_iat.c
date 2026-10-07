/* Pure PE fixtures and import changes only in this standalone test process. */
#include "iat_data.h"
#include <assert.h>
#include <stdio.h>
__declspec(dllimport) int __cdecl cef_browser_host_create_browser_sync(void);
static int __cdecl replacement_create(void){return 91;}
/* Force a fresh import load for each fixture call; an optimized C caller may
 * otherwise cache an imported function pointer in a register. */
__attribute__((noinline,noipa)) static int invoke_import(void){return cef_browser_host_create_browser_sync();}
static void synthetic_image(BYTE* image,SIZE_T size){
 memset(image,0,size);
 IMAGE_DOS_HEADER* dos=(IMAGE_DOS_HEADER*)image;dos->e_magic=IMAGE_DOS_SIGNATURE;dos->e_lfanew=0x80;
 IMAGE_NT_HEADERS64* nt=(IMAGE_NT_HEADERS64*)(image+0x80);nt->Signature=IMAGE_NT_SIGNATURE;
 nt->FileHeader.Machine=IMAGE_FILE_MACHINE_AMD64;nt->FileHeader.SizeOfOptionalHeader=sizeof(IMAGE_OPTIONAL_HEADER64);
 nt->OptionalHeader.Magic=IMAGE_NT_OPTIONAL_HDR64_MAGIC;nt->OptionalHeader.SizeOfImage=(DWORD)size;nt->OptionalHeader.NumberOfRvaAndSizes=IMAGE_NUMBEROF_DIRECTORY_ENTRIES;
 nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT]=(IMAGE_DATA_DIRECTORY){0x200,2*sizeof(IMAGE_IMPORT_DESCRIPTOR)};
 IMAGE_IMPORT_DESCRIPTOR* imports=(IMAGE_IMPORT_DESCRIPTOR*)(image+0x200);imports->Name=0x300;imports->OriginalFirstThunk=0x400;imports->FirstThunk=0x500;
 memcpy(image+0x300,"LIBCEF.DLL",11);memcpy(image+0x342,"cef_browser_host_create_browser_sync",36);
 IMAGE_THUNK_DATA64* names=(IMAGE_THUNK_DATA64*)(image+0x400);names[0].u1.AddressOfData=0x340;
}
int main(void){
 union {uint64_t align;BYTE bytes[4096];} storage;BYTE* image=storage.bytes;void** slot=NULL;
 synthetic_image(image,sizeof(storage));assert(iat_find_cef_slot(image,sizeof(storage),&slot)==IAT_OK);assert(slot==(void**)(image+0x500));
 puts("PASS: exact named libcef PE32+ import found in a bounded synthetic image");
 IMAGE_NT_HEADERS64* nt=(IMAGE_NT_HEADERS64*)(image+0x80);nt->FileHeader.Machine=IMAGE_FILE_MACHINE_I386;assert(iat_find_cef_slot(image,sizeof(storage),&slot)==IAT_BAD_IMAGE);
 synthetic_image(image,sizeof(storage));nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress=0xfff;assert(iat_find_cef_slot(image,sizeof(storage),&slot)==IAT_BAD_IMPORTS);
 synthetic_image(image,sizeof(storage));((IMAGE_IMPORT_DESCRIPTOR*)(image+0x200))->OriginalFirstThunk=0;assert(iat_find_cef_slot(image,sizeof(storage),&slot)==IAT_BAD_IMPORTS);
 synthetic_image(image,sizeof(storage));memcpy(image+0x342,"other_browser_function",23);assert(iat_find_cef_slot(image,sizeof(storage),&slot)==IAT_IMPORT_NOT_FOUND);
 synthetic_image(image,sizeof(storage));((IMAGE_THUNK_DATA64*)(image+0x400))[1].u1.AddressOfData=0x340;assert(iat_find_cef_slot(image,sizeof(storage),&slot)==IAT_DUPLICATE_IMPORT);
 puts("PASS: wrong architecture, malformed bounds, absent names and duplicate imports rejected");
 DWORD error=0;
 void* private_slot=(void*)1;assert(iat_replace_data_slot(&private_slot,(void*)1,(void*)2,&error)==IAT_PAGE_NOT_DATA);assert(private_slot==(void*)1);
 void** executable_address=(void**)((uintptr_t)(void*)replacement_create & ~((uintptr_t)sizeof(void*)-1));
 assert(iat_replace_data_slot(executable_address,(void*)1,(void*)2,&error)==IAT_PAGE_NOT_DATA);
 puts("PASS: private and executable pages rejected without changing memory protection");
 BYTE* own=(BYTE*)GetModuleHandleW(NULL);SIZE_T size=iat_own_image_size(own);assert(size);
 assert(iat_find_cef_slot(own,size,&slot)==IAT_OK);HMODULE fixture=GetModuleHandleW(L"libcef.dll");assert(fixture);
 void* original=(void*)GetProcAddress(fixture,"cef_browser_host_create_browser_sync");assert(original && *slot==original);
 MEMORY_BASIC_INFORMATION before,after;assert(VirtualQuery(slot,&before,sizeof(before)));assert(invoke_import()==37);
 assert(iat_replace_data_slot(slot,(void*)1,(void*)replacement_create,&error)==IAT_POINTER_UNEXPECTED);assert(*slot==original);
 assert(iat_replace_data_slot(slot,original,(void*)replacement_create,&error)==IAT_OK);assert(invoke_import()==91);
 assert(VirtualQuery(slot,&after,sizeof(after)));assert(after.Protect==before.Protect && after.Type==MEM_IMAGE);
 assert(iat_replace_data_slot(slot,original,(void*)replacement_create,&error)==IAT_POINTER_UNEXPECTED);
 assert(iat_replace_data_slot(slot,(void*)replacement_create,original,&error)==IAT_OK);assert(invoke_import()==37);
 assert(VirtualQuery(slot,&after,sizeof(after)));assert(after.Protect==before.Protect);
 puts("PASS: own-process public data import changes atomically, expected-pointer guards hold, original page protection and pointer restore");
 return 0;
}