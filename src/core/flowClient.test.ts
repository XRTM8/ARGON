import {beforeEach,afterEach,describe,it,expect,vi} from 'vitest';
import {FlowClient} from './flowClient';
import type {FlowSnapshot} from '../types/flow';
const {invoke,listen,unlisten}=vi.hoisted(()=>({invoke:vi.fn(),listen:vi.fn(),unlisten:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke}));
vi.mock('@tauri-apps/api/event',()=>({listen}));
const snapshot:FlowSnapshot={enabled:false,id:'a'.repeat(64),name:'This device',port:0,addresses:[],receiveDirectory:'C:\\Received',peers:[],transfers:[],pairings:[],incoming:[],clipboard:[],lastError:null,protocol:'argon-flow/2'};
let client:FlowClient,release:()=>void;
beforeEach(()=>{
  vi.stubGlobal('window',{__TAURI_INTERNALS__:{}});invoke.mockReset();listen.mockReset();unlisten.mockReset();
  listen.mockResolvedValue(unlisten);invoke.mockImplementation(async(command)=>command==='flow_snapshot'?structuredClone(snapshot):undefined);
  client=new FlowClient();release=client.acquire();
});
afterEach(()=>{release();vi.unstubAllGlobals();});
describe('native Flow client',()=>{
  it('waits for native events and loads real state instead of demo devices',async()=>{
    await vi.waitFor(()=>expect(client.getState().snapshot?.protocol).toBe('argon-flow/2'));
    expect(listen).toHaveBeenCalledWith('flow-event',expect.any(Function));expect(client.getState().snapshot?.peers).toEqual([]);
  });
  it('shares one native listener between mounted views and cleans it up once',async()=>{
    const another=client.acquire();await vi.waitFor(()=>expect(client.getState().snapshot).not.toBeNull());
    release();expect(unlisten).not.toHaveBeenCalled();another();another();expect(unlisten).toHaveBeenCalledTimes(1);
  });
  it('never invokes sharing commands in browser preview mode',async()=>{
    await vi.waitFor(()=>expect(client.getState().snapshot).not.toBeNull());release();invoke.mockClear();vi.stubGlobal('window',{});
    await expect(client.execute('flow_start')).rejects.toThrow('ARGON');expect(invoke).not.toHaveBeenCalled();
  });
  it('passes only picker-issued file IDs into a transfer command',async()=>{
    await vi.waitFor(()=>expect(client.getState().snapshot).not.toBeNull());
    invoke.mockImplementation(async(command)=>command==='flow_choose_files'?[{id:'picker-token',name:'file.pdf',size:100}]:command==='flow_snapshot'?snapshot:[]);
    client.selectPeer('trusted-peer');await client.chooseFiles('C:\\Downloads\\file.pdf');await client.sendFiles();
    expect(invoke).toHaveBeenCalledWith('flow_send_files',{peerId:'trusted-peer',fileIds:['picker-token']});expect(client.getState().selectedFiles).toEqual([]);
  });
  it('preserves a selection if the native picker is cancelled',async()=>{
    await vi.waitFor(()=>expect(client.getState().snapshot).not.toBeNull());
    invoke.mockResolvedValueOnce([{id:'selected',name:'file.pdf',size:5}]);await client.chooseFiles();invoke.mockResolvedValueOnce([]);await client.chooseFiles();
    expect(client.getState().selectedFiles).toHaveLength(1);
  });
  it('keeps native failures visible and does not send arbitrary paths on failure',async()=>{
    await vi.waitFor(()=>expect(client.getState().snapshot).not.toBeNull());invoke.mockRejectedValueOnce('Pair the device first');
    await expect(client.execute('flow_send_text',{peerId:'unknown',text:'exact?token=keep'})).rejects.toBe('Pair the device first');
    expect(client.getState().error).toBe('Pair the device first');expect(client.getState().pending).toEqual([]);
  });
});
