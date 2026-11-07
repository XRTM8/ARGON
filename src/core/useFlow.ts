import {useEffect,useSyncExternalStore} from 'react';
import {flowClient} from './flowClient';
export function useFlow(){
  const state=useSyncExternalStore(flowClient.subscribe,flowClient.getState);
  useEffect(()=>flowClient.acquire(),[]);
  return {...state,client:flowClient};
}
