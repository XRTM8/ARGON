import { DEFAULTS, httpUrl, excludedHost, platformVideo, scanPageLinks } from './core.js';
import { buildAuthHeaders, connection, sendDownload } from './bridge-client.js';
const $ = id => document.getElementById(id);
let settings = await chrome.storage.local.get(DEFAULTS), language = settings.language === 'en' ? 'en' : 'ar';
let tab, media = [], pageFiles = [], online = false, hasFfmpeg = false, busy = false;
const sent = new Set();
const t = (ar,en) => language === 'ar' ? ar : en;
function message(text='',error=false){$('feedback').textContent=text;$('feedback').className=error?'error':'';}
function labels(){
 document.documentElement.lang=language;document.documentElement.dir=language==='ar'?'rtl':'ltr';$('language').textContent=t('EN','عربي');
 for(const [id,ar,en]of [['optionsLabel','خيارات','Options'],['captureLabel','التقاط التنزيلات تلقائياً','Capture browser downloads'],['videoLabel','إظهار زر التنزيل على الفيديو','Show video download button'],['cookiesLabel','تمرير الكوكيز للمواقع المحمية','Forward cookies for gated sites'],['minSizeLabel','أقل حجم للالتقاط','Minimum capture size'],['anySize','أي حجم','Any size'],['manualLabel','رابط يدوي','Manual link'],['sendBtn','تنزيل','Download'],['privacyLabel','اكتشاف محلي','Local discovery']])$(id).textContent=t(ar,en);
 $('version').textContent=`v${chrome.runtime.getManifest().version}`;
 $('checkStatus').setAttribute('aria-label',t('فحص الاتصال','Check connection'));$('scanPage').setAttribute('aria-label',t('تحديث الملفات المكتشفة','Refresh detected files'));
 const host=httpUrl(tab?.url)?.hostname;$('excludeSite').hidden=!host;
 if(host)$('excludeSite').textContent=t(excludedHost(tab.url,settings.excludedHosts)?'إلغاء استثناء الموقع':'استثناء الموقع',excludedHost(tab.url,settings.excludedHosts)?'Enable this site':'Exclude this site')+` · ${host}`;
 render();
}
function items(){
 const platform=platformVideo(tab?.url,tab?.title);
 const found=new Map();
 for(const item of [...(platform?[platform]:media.filter(item=>item.supported||(hasFfmpeg&&item.playlist))),...pageFiles.filter(item=>!platform||!item.media)])if(!found.has(item.url))found.set(item.url,item);
 return [...found.values()].slice(0,40);
}
function render(){
 const files=items();$('filesHeading').textContent=t('ملفات الصفحة','Page downloads')+(files.length?` (${files.length})`:'');$('mediaList').replaceChildren();$('emptyMedia').hidden=!!files.length;
 $('emptyMedia').textContent=httpUrl(tab?.url)?t('شغّل الفيديو أو افتح صفحة فيها ملفات. تظهر التنزيلات هنا تلقائياً.','Play a video or open a page with file links. Downloads appear here automatically.'):t('افتح صفحة فيديو أو ملفات؛ هذه صفحة إعدادات المتصفح.','Open a video or file page. Browser settings cannot be scanned.');
 for(const item of files){
  const row=document.createElement('div');row.className='file-item';const info=document.createElement('div');info.className='file-info';const title=document.createElement('span');title.className='file-title';title.dir='auto';title.textContent=item.filename;title.title=item.filename;
  const meta=document.createElement('span');meta.className='file-type';meta.textContent=`${item.type}${item.playlist?' · FFmpeg':''} · ${httpUrl(item.url)?.hostname||''}`;info.append(title,meta);
  const button=document.createElement('button');button.className='primary';button.textContent=sent.has(item.url)?t('أُضيف','Added'):t('تنزيل','Download');button.disabled=busy||!online||sent.has(item.url);button.addEventListener('click',()=>void send(item));row.append(info,button);$('mediaList').append(row);
 }
}
async function check(){
 $('checkStatus').disabled=true;$('statusText').textContent=t('جارٍ الاتصال…','Connecting…');
 try{const ping=await connection();online=true;hasFfmpeg=!!ping.capabilities?.includes('ffmpeg-hls');$('statusText').textContent=t('متصل بـ ARGON','Connected to ARGON');$('statusDot').className='dot online';$('statusHelp').hidden=true;}
 catch(error){online=false;hasFfmpeg=false;$('statusText').textContent=t('شغّل ARGON للتنزيل','Open ARGON to download');$('statusDot').className='dot';$('statusHelp').hidden=error.message!=='update_required';$('statusHelp').textContent=t('حدّث تطبيق ARGON أولاً.','Update the ARGON app first.');}
 finally{$('checkStatus').disabled=false;render();}
}
async function send(item){
 if(busy)return;busy=true;render();
 try{const ping=await connection();const allowPlaylists=!!ping.capabilities?.includes('ffmpeg-hls');const headers=await buildAuthHeaders(item.url,tab?.url,settings,!!tab?.incognito);await sendDownload(item.url,item.platform?undefined:item.filename,tab?.url,headers,undefined,undefined,allowPlaylists);sent.add(item.url);message(t('أُضيف إلى ARGON.','Added to ARGON.'));}
 catch{message(t('تعذّر التسليم؛ شغّل ARGON وأعد المحاولة.','Could not hand off. Open ARGON and try again.'),true);}
 finally{busy=false;render();void check();}
}
async function scan(){
 if(tab?.id===undefined||!httpUrl(tab.url))return;
 try{const result=await chrome.scripting.executeScript({target:{tabId:tab.id},func:scanPageLinks});pageFiles=result[0]?.result||[];render();}catch{/* Restricted browser pages cannot be scanned. */}
}
$('checkStatus').addEventListener('click',check);$('scanPage').addEventListener('click',()=>void scan());
$('language').addEventListener('click',async()=>{language=language==='ar'?'en':'ar';await chrome.storage.local.set({language});labels();void check();});
$('interceptToggle').checked=settings.autoIntercept;$('videoToggle').checked=settings.showVideoButton;$('cookiesToggle').checked=!!settings.forwardCookies;
for(const[id,key]of [['interceptToggle','autoIntercept'],['videoToggle','showVideoButton']])$(id).addEventListener('change',async()=>{settings[key]=$(id).checked;await chrome.storage.local.set({[key]:settings[key]});});
$('cookiesToggle').addEventListener('change',async()=>{
 const want=$('cookiesToggle').checked;
 if(want && chrome.permissions?.request){
  const granted=await chrome.permissions.request({permissions:['cookies']}).catch(()=>false);
  if(!granted){$('cookiesToggle').checked=false;settings.forwardCookies=false;await chrome.storage.local.set({forwardCookies:false});return;}
 }
 settings.forwardCookies=want;await chrome.storage.local.set({forwardCookies:want});
});
$('minSize').value=String(settings.minSizeBytes);$('minSize').addEventListener('change',async()=>{settings.minSizeBytes=Number($('minSize').value);await chrome.storage.local.set({minSizeBytes:settings.minSizeBytes});});
$('excludeSite').addEventListener('click',async()=>{const host=httpUrl(tab?.url)?.hostname;if(!host)return;settings.excludedHosts=excludedHost(tab.url,settings.excludedHosts)?settings.excludedHosts.filter(rule=>!excludedHost(tab.url,[rule])):[...settings.excludedHosts,host];await chrome.storage.local.set({excludedHosts:settings.excludedHosts});labels();});
$('manualForm').addEventListener('submit',event=>{event.preventDefault();const url=$('manualUrl').value.trim();if(httpUrl(url))void send({url});});
try{[tab]=await chrome.tabs.query({active:true,currentWindow:true});}catch{/* No web tab is available. */}
labels();void check();void scan();
if(tab?.id!==undefined){
 try{media=(await chrome.runtime.sendMessage({type:'GET_TAB_MEDIA',tabId:tab.id}))?.media||[];render();}catch{}
 chrome.storage.onChanged.addListener((changes,area)=>{if(area==='session'&&changes[`media:${tab.id}`]){media=changes[`media:${tab.id}`].newValue||[];render();}});
}
