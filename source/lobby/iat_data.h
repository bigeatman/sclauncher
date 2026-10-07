/* Only the named public libcef import in the calling EXE is replaced.
 * No executable allocation, trampoline, machine-code change, mitigation-policy
 * change or remote-process inspection is used by this implementation. */
#ifndef SC_CEF_IAT_DATA_H
#define SC_CEF_IAT_DATA_H
#include <windows.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

enum iat_result {
 IAT_OK=0, IAT_BAD_IMAGE=1, IAT_BAD_IMPORTS=2, IAT_IMPORT_NOT_FOUND=3,
 IAT_DUPLICATE_IMPORT=4, IAT_POINTER_UNEXPECTED=5, IAT_PAGE_NOT_DATA=6,
 IAT_PROTECT_FAILED=7, IAT_POINTER_CHANGED=8, IAT_RESTORE_FAILED=9
};
static int iat_range(SIZE_T size,SIZE_T offset,SIZE_T length){return offset<=size && length<=size-offset;}
static int iat_ascii_equal(const char* value,const char* expected,int fold_case){
 for(size_t i=0;;i++){
  unsigned char a=(unsigned char)value[i],b=(unsigned char)expected[i];
  if(fold_case){if(a>='A' && a<='Z')a+=32;if(b>='A' && b<='Z')b+=32;}
  if(a!=b)return 0;
  if(!a)return 1;
 }
}
static const char* iat_string(BYTE* image,SIZE_T size,DWORD rva,SIZE_T cap){
 if(!iat_range(size,rva,1))return NULL;
 SIZE_T left=size-rva;if(left>cap)left=cap;
 for(SIZE_T i=0;i<left;i++)if(!image[rva+i])return (const char*)image+rva;
 return NULL;
}
/* Input is a mapped, trusted PE32+ image owned by this process, or a synthetic
 * bounded fixture. Names are checked against OriginalFirstThunk, never guessed
 * from code bytes or another process's memory. */
static enum iat_result iat_find_cef_slot(BYTE* image,SIZE_T size,void*** slot_out){
 if(!image || !slot_out || size<sizeof(IMAGE_DOS_HEADER))return IAT_BAD_IMAGE;
 *slot_out=NULL;
 const IMAGE_DOS_HEADER* dos=(const IMAGE_DOS_HEADER*)image;
 if(dos->e_magic!=IMAGE_DOS_SIGNATURE || dos->e_lfanew<0 || !iat_range(size,(DWORD)dos->e_lfanew,sizeof(IMAGE_NT_HEADERS64)))return IAT_BAD_IMAGE;
 const IMAGE_NT_HEADERS64* nt=(const IMAGE_NT_HEADERS64*)(image+(DWORD)dos->e_lfanew);
 if(nt->Signature!=IMAGE_NT_SIGNATURE || nt->FileHeader.Machine!=IMAGE_FILE_MACHINE_AMD64 || nt->FileHeader.SizeOfOptionalHeader<sizeof(IMAGE_OPTIONAL_HEADER64) || nt->OptionalHeader.Magic!=IMAGE_NT_OPTIONAL_HDR64_MAGIC || nt->OptionalHeader.SizeOfImage!=size || nt->OptionalHeader.NumberOfRvaAndSizes<=IMAGE_DIRECTORY_ENTRY_IMPORT)return IAT_BAD_IMAGE;
 IMAGE_DATA_DIRECTORY directory=nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT];
 if(!directory.VirtualAddress || directory.Size<sizeof(IMAGE_IMPORT_DESCRIPTOR) || !iat_range(size,directory.VirtualAddress,directory.Size))return IAT_BAD_IMPORTS;
 SIZE_T descriptors=directory.Size/sizeof(IMAGE_IMPORT_DESCRIPTOR);
 const IMAGE_IMPORT_DESCRIPTOR* imports=(const IMAGE_IMPORT_DESCRIPTOR*)(image+directory.VirtualAddress);
 int terminated=0;
 for(SIZE_T d=0;d<descriptors;d++){
  const IMAGE_IMPORT_DESCRIPTOR* item=imports+d;
  if(!item->Name && !item->FirstThunk && !item->OriginalFirstThunk){terminated=1;break;}
  const char* library=iat_string(image,size,item->Name,256);
  if(!library)return IAT_BAD_IMPORTS;
  if(!iat_ascii_equal(library,"libcef.dll",1))continue;
  if(!item->OriginalFirstThunk || !item->FirstThunk || item->FirstThunk%sizeof(void*) || item->OriginalFirstThunk%sizeof(uint64_t))return IAT_BAD_IMPORTS;
  SIZE_T limit=(size-item->OriginalFirstThunk)/sizeof(IMAGE_THUNK_DATA64);
  if(!iat_range(size,item->OriginalFirstThunk,sizeof(IMAGE_THUNK_DATA64)))return IAT_BAD_IMPORTS;
  int thunk_terminated=0;
  for(SIZE_T t=0;t<limit;t++){
   SIZE_T lookup_rva=(SIZE_T)item->OriginalFirstThunk+t*sizeof(IMAGE_THUNK_DATA64);
   SIZE_T slot_rva=(SIZE_T)item->FirstThunk+t*sizeof(IMAGE_THUNK_DATA64);
   if(!iat_range(size,slot_rva,sizeof(void*)))return IAT_BAD_IMPORTS;
   const IMAGE_THUNK_DATA64* lookup=(const IMAGE_THUNK_DATA64*)(image+lookup_rva);
   ULONGLONG name_rva=lookup->u1.AddressOfData;
   if(!name_rva){thunk_terminated=1;break;}
   if(IMAGE_SNAP_BY_ORDINAL64(name_rva))continue;
   if(name_rva>MAXDWORD || !iat_range(size,(SIZE_T)name_rva,sizeof(WORD)+1))return IAT_BAD_IMPORTS;
   const char* name=iat_string(image,size,(DWORD)name_rva+sizeof(WORD),512);
   if(!name)return IAT_BAD_IMPORTS;
   if(!iat_ascii_equal(name,"cef_browser_host_create_browser_sync",0))continue;
   if(*slot_out)return IAT_DUPLICATE_IMPORT;
   *slot_out=(void**)(image+slot_rva);
  }
  if(!thunk_terminated)return IAT_BAD_IMPORTS;
 }
 if(!terminated)return IAT_BAD_IMPORTS;
 return *slot_out?IAT_OK:IAT_IMPORT_NOT_FOUND;
}
static SIZE_T iat_own_image_size(BYTE* image){
 MEMORY_BASIC_INFORMATION info;
 if(!image || !VirtualQuery(image,&info,sizeof(info)) || info.State!=MEM_COMMIT || info.Type!=MEM_IMAGE || info.AllocationBase!=(void*)image || info.RegionSize<sizeof(IMAGE_DOS_HEADER))return 0;
 const IMAGE_DOS_HEADER* dos=(const IMAGE_DOS_HEADER*)image;
 if(dos->e_magic!=IMAGE_DOS_SIGNATURE || dos->e_lfanew<0 || !iat_range(info.RegionSize,(DWORD)dos->e_lfanew,sizeof(IMAGE_NT_HEADERS64)))return 0;
 const IMAGE_NT_HEADERS64* nt=(const IMAGE_NT_HEADERS64*)(image+(DWORD)dos->e_lfanew);
 if(nt->Signature!=IMAGE_NT_SIGNATURE || nt->OptionalHeader.Magic!=IMAGE_NT_OPTIONAL_HDR64_MAGIC)return 0;
 return nt->OptionalHeader.SizeOfImage;
}
/* A normal writable/read-only MEM_IMAGE data page is required. RX, RWX,
 * executable-copy, guarded, no-access and private pages are never modified. */
static enum iat_result iat_replace_data_slot(void** slot,void* expected,void* replacement,DWORD* win32_out){
 MEMORY_BASIC_INFORMATION info;DWORD old_protection=0,ignored=0;
 if(win32_out)*win32_out=0;
 if(!slot || (uintptr_t)slot%sizeof(void*) || !expected || !replacement)return IAT_POINTER_UNEXPECTED;
 if(!VirtualQuery(slot,&info,sizeof(info))){if(win32_out)*win32_out=GetLastError();return IAT_PAGE_NOT_DATA;}
 DWORD protection=info.Protect;
 if(info.State!=MEM_COMMIT || info.Type!=MEM_IMAGE || (protection!=PAGE_READONLY && protection!=PAGE_READWRITE && protection!=PAGE_WRITECOPY) || (uintptr_t)slot<(uintptr_t)info.BaseAddress || sizeof(void*)>info.RegionSize-((uintptr_t)slot-(uintptr_t)info.BaseAddress))return IAT_PAGE_NOT_DATA;
 if(*slot!=expected)return IAT_POINTER_UNEXPECTED;
 if(!VirtualProtect(slot,sizeof(void*),PAGE_READWRITE,&old_protection)){if(win32_out)*win32_out=GetLastError();return IAT_PROTECT_FAILED;}
 void* previous=InterlockedCompareExchangePointer(slot,replacement,expected);
 if(!VirtualProtect(slot,sizeof(void*),old_protection,&ignored)){
  DWORD error=GetLastError();
  /* Roll back only our own pointer if the protection restoration failed. */
  InterlockedCompareExchangePointer(slot,expected,replacement);
  VirtualProtect(slot,sizeof(void*),old_protection,&ignored);
  if(win32_out)*win32_out=error;
  return IAT_RESTORE_FAILED;
 }
 return previous==expected?IAT_OK:IAT_POINTER_CHANGED;
}
#endif