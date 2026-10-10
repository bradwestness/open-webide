import runpy,json,sys
module=runpy.run_path('/workspace/repos/openwebide/tools/measure-editor-view.py')
Original=module['Browser']
PATCH=r"""
(() => {
 const native=Object.getOwnPropertyDescriptor(Element.prototype,'innerHTML');

 window.probeRetention={same:0,replaced:0,cleared:0,textRetained:0,textChanged:0};
 const sync=(old,fresh)=>{
  if(old.isEqualNode(fresh)){probeRetention.same++;if(old.nodeType===3)probeRetention.textRetained+=old.length;return;}
  if(old.nodeType!==fresh.nodeType || old.nodeName!==fresh.nodeName){old.replaceWith(fresh);probeRetention.replaced++;return;}
  if(old.nodeType===3){probeRetention.textChanged+=fresh.length;old.nodeValue=fresh.nodeValue;return;}
  for(const name of old.getAttributeNames())if(!fresh.hasAttribute(name))old.removeAttribute(name);
  for(const name of fresh.getAttributeNames())if(old.getAttribute(name)!==fresh.getAttribute(name))old.setAttribute(name,fresh.getAttribute(name));
  const children=[...fresh.childNodes];
  while(old.childNodes.length>children.length)old.lastChild.remove();
  children.forEach((child,index)=>{const previous=old.childNodes[index];if(previous)sync(previous,child);else old.appendChild(child);});
 };
 Object.defineProperty(Element.prototype,'innerHTML',{...native,set(html){
  if (!this.matches('.editor-row-measure > .editor-highlight-content')) return native.set.call(this,html);
  if (!html) {probeRetention.cleared++; return;}
  const box=document.createElement('div');native.set.call(box,html);
  const incoming=box.firstElementChild;
  const old=this.firstElementChild;
  if(incoming && old && this.childNodes.length===1 && box.childNodes.length===1) {
   const gap=old.firstElementChild;
   if(gap && gap.getAttribute('style')?.startsWith('display:inline-block;width:'))gap.remove();
   sync(old,incoming);
   if(native.get.call(this)!==html)throw Error('DOM reconciliation differs from rendered HTML');
  }else{native.set.call(this,html);probeRetention.replaced++;}
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
   if self.session:print(json.dumps({'experiment':'exact differential probe DOM experiment','counts':self.script('return window.probeRetention')}),file=sys.stderr,flush=True)
  finally:super().stop()
module['measure'].__globals__['Browser']=Browser
for mode in ['local','remote']:
 print(json.dumps(module['measure']('styled-long-line',mode,True,True,1,'beginning')),flush=True)
