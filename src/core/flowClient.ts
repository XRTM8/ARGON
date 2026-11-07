import type { FlowSnapshot, FlowSelectedFile } from '../types/flow';

export function nativeRuntime():boolean { return typeof window!=='undefined' && '__TAURI_INTERNALS__' in window; }
export interface FlowClientState {
  snapshot:FlowSnapshot|null; error:string|null; pending:string[]; selectedFiles:FlowSelectedFile[]; selectedPeerId:string;
}

export class FlowClient {
  private state:FlowClientState={snapshot:null,error:null,pending:[],selectedFiles:[],selectedPeerId:''};
  private listeners=new Set<()=>void>();
  private references=0;
  private generation=0;
  private revision=0;
  private polling:ReturnType<typeof setInterval>|null=null;
  private unlisten:(()=>void)|null=null;
  private refreshing=false;
  private trailing:ReturnType<typeof setTimeout>|null=null;
  private dismissedError:string|null=null;
  public subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
  public getState=()=>this.state;
  private set(update:Partial<FlowClientState>) { this.state={...this.state,...update};this.listeners.forEach(fn=>fn()); }
  public clearError=()=>{this.dismissedError=this.state.snapshot?.lastError||this.state.error;this.set({error:null});};
  public selectPeer=(id:string)=>this.set({selectedPeerId:id});
  public removeSelected=(id:string)=>this.set({selectedFiles:this.state.selectedFiles.filter(file=>file.id!==id)});
  public acquire=()=>{
    this.references++;
    if(this.references===1) void this.initialize(++this.generation);
    let released=false;
    return()=>{
      if(released)return;released=true;
      this.references--;
      if(this.references===0) {
        this.generation++;this.unlisten?.();this.unlisten=null;
        if(this.polling) clearInterval(this.polling);this.polling=null;
        if(this.trailing) clearTimeout(this.trailing);this.trailing=null;
      }
    };
  };
  private async initialize(generation:number) {
    if(!nativeRuntime()) return;
    try {
      const {listen}=await import('@tauri-apps/api/event');
      const unlisten=await listen<{type:string;data?:unknown}>('flow-event',event=>{
        if(generation!==this.generation) return;
        if(event.payload.type==='error') {this.dismissedError=null;this.set({error:String(event.payload.data)});}
        this.scheduleRefresh();
      });
      if(generation!==this.generation) {unlisten();return;}
      this.unlisten=unlisten;await this.refresh();
      if(generation!==this.generation) return;
      this.polling=setInterval(()=>{void this.refresh();},1500);
    } catch(error) {if(generation===this.generation)this.set({error:String(error)});}
  }
  private scheduleRefresh() {
    if(this.trailing) return;
    this.trailing=setTimeout(()=>{this.trailing=null;void this.refresh();},100);
  }
  public async refresh():Promise<void> {
    if(!nativeRuntime() || this.refreshing) return;
    this.refreshing=true;const revision=this.revision;const generation=this.generation;
    try {
      const {invoke}=await import('@tauri-apps/api/core');const snapshot=await invoke<FlowSnapshot>('flow_snapshot');
      if(revision===this.revision && generation===this.generation) {
        const selected=this.state.selectedPeerId;
        const peer=snapshot.peers.find(p=>p.id===selected)||snapshot.peers.find(p=>p.trusted)||snapshot.peers[0];
        this.set({snapshot,selectedPeerId:peer?.id||'',...(snapshot.lastError&&snapshot.lastError!==this.dismissedError?{error:snapshot.lastError}:{})});
      }
    }catch(error){if(generation===this.generation)this.set({error:String(error)});}
    finally {this.refreshing=false;}
  }
  public async execute<T=void>(command:string,args?:Record<string,unknown>):Promise<T> {
    if(!nativeRuntime()){const error='المشاركة تتطلب تطبيق ARGON المثبّت.';this.set({error});throw new Error(error);}
    if(this.state.pending.includes(command))throw new Error('This action is already in progress');
    this.revision++;
    this.dismissedError=this.state.snapshot?.lastError||null;
    this.set({error:null,pending:[...this.state.pending,command]});
    try {
      const {invoke}=await import('@tauri-apps/api/core');return await invoke<T>(command,args);
    }catch(error){this.set({error:String(error)});throw error;}
    finally {
      this.set({pending:this.state.pending.filter(c=>c!==command)});
      // A background read started before this command must not overwrite its result.
      this.revision++;this.scheduleRefresh();
    }
  }
  public async chooseFiles(hint?:string) {
    const files=await this.execute<FlowSelectedFile[]>('flow_choose_files',{hint:hint||null});
    if(files.length)this.set({selectedFiles:files});return files;
  }
  public async sendFiles() {
    const ids=await this.execute<string[]>('flow_send_files',{peerId:this.state.selectedPeerId,fileIds:this.state.selectedFiles.map(file=>file.id)});
    this.set({selectedFiles:[]});return ids;
  }
}
export const flowClient=new FlowClient();
