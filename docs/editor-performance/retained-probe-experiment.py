import runpy,json,sys
module=runpy.run_path('/workspace/repos/openwebide/tools/measure-editor-view.py')
Original=module['Browser']
PATCH=r"""
(() => {
 const native=Object.getOwnPropertyDescriptor(Element.prototype,'innerHTML');
 const retained=new WeakMap();
 window.probeRetention={same:0,replaced:0,cleared:0};
 Object.defineProperty(Element.prototype,'innerHTML',{...native,set(html){
  if (!this.matches('.editor-row-measure > .editor-highlight-content')) return native.set.call(this,html);
  if (!html) {probeRetention.cleared++; return;}
  const box=document.createElement('div');native.set.call(box,html);
  const incoming=box.firstElementChild;
  const old=this.firstElementChild;
  const previous=retained.get(this);
  if(incoming && old && previous===native.get.call(incoming)) {
   const gap=old.firstElementChild;
   if(gap && gap.getAttribute('style')?.startsWith('display:inline-block;width:'))gap.remove();
   for(const name of old.getAttributeNames())if(!incoming.hasAttribute(name))old.removeAttribute(name);
   for(const name of incoming.getAttributeNames())if(old.getAttribute(name)!==incoming.getAttribute(name))old.setAttribute(name,incoming.getAttribute(name));
   probeRetention.same++;
  }else{
   native.set.call(this,html);retained.set(this,incoming ? native.get.call(incoming):null);probeRetention.replaced++;
  }
 }});
})();
"""
class Browser(Original):
 def call(self,method,path,body=None,session=True,timeout=30):
  if path=='/goog/cdp/execute' and body and body.get('cmd')=='Page.addScriptToEvaluateOnNewDocument':
   body={**body,'params':{**body['params'],'source':PATCH+body['params']['source']}}
  return super().call(method,path,body,session,timeout)
 def stop(self):
  try:
   if self.session:print(json.dumps({'experiment':'retained identical probe children','counts':self.script('return window.probeRetention')}),file=sys.stderr,flush=True)
  finally:super().stop()
module['measure'].__globals__['Browser']=Browser
for mode in ['local','remote']:
 print(json.dumps(module['measure']('styled-long-line',mode,True,True,1,'beginning')),flush=True)
