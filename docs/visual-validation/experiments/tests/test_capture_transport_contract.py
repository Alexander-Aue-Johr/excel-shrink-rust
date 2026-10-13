"""Source/manifest contracts only. Does not execute PowerShell or compile native hosts."""
from pathlib import Path
import unittest
from xml.etree import ElementTree as ET
ROOT=Path(__file__).resolve().parents[1]
OFFICE=ROOT/'_shared/office-addin'
class Contracts(unittest.TestCase):
 def test_rust_is_http_only_and_loopback(self):
  src=(OFFICE/'hosts/rust/src/main.rs').read_text();toml=(OFFICE/'hosts/rust/Cargo.toml').read_text()
  for symbol in ['RustlsConfig','ensure_cert','generate_simple_self_signed','axum_server','ensure-cert']: self.assertNotIn(symbol,src)
  for dep in ['axum-server','rcgen']:self.assertNotIn(dep,toml)
  self.assertIn('([127, 0, 0, 1], 3000)',src);self.assertIn('([127, 0, 0, 1], 3001)',src)
  self.assertIn('sec-fetch-site',src);self.assertIn('X-VV-Run-Id',src);self.assertIn('header::HOST',src);self.assertIn('header::ORIGIN',src)
 def test_actual_manifest_transform_preserves_external_https(self):
  original=(OFFICE/'web/manifest.xml').read_text();rust=original.replace('https://localhost:3000','http://localhost:3000').replace('46f974c9-b264-4b71-bbb5-670f65ec1749','7b0dcce4-d6e3-4c36-bc37-5c4a77c3cbe2')
  doc=ET.fromstring(rust);self.assertNotIn('https://localhost:3000',rust);self.assertIn('http://localhost:3000/taskpane.html',rust)
  ns={'o':'http://schemas.microsoft.com/office/appforoffice/1.1'}
  self.assertEqual(doc.find('o:Id',ns).text,'7b0dcce4-d6e3-4c36-bc37-5c4a77c3cbe2')
  self.assertRegex((OFFICE/'web/taskpane.html').read_text(), r'<script src="https://[^"]+/office\.js"')
 def test_runner_removes_only_rust_cert_operations(self):
  s=(OFFICE/'run.ps1').read_text();part=s[s.index('function Ensure-RustHost'):s.index('function Get-ListeningProcessIds')]
  self.assertNotIn('ensure-cert',part);self.assertNotIn('X509',part);self.assertNotIn('$store.Add',part);self.assertIn("Args = @('serve')",part)
  self.assertIn('dev-certs https',s);self.assertIn('VV_EXCEL_LAUNCH_MODE',s);self.assertIn('inputs/host/',s)
 def test_helpers_and_guards_reachable_on_both_hosts(self):
  for rel in ['hosts/rust/src/main.rs','hosts/dotnet/Program.cs']:
   s=(OFFICE/rel).read_text();self.assertIn('/capture-policy.js',s);self.assertIn('/finalize-image',s);self.assertIn('X-VV-Run-Id',s)
  self.assertIn('FinalImageEndpoints.Map',(OFFICE/'hosts/dotnet/Program.cs').read_text())
 def test_native_crop_validates_source_and_preserves_raw(self):
  for rel in ['hosts/rust/src/final_image.rs','hosts/dotnet/FinalImageEndpoints.cs']:
   s=(OFFICE/rel).read_text();self.assertIn('source_sha256',s);self.assertIn('source_width',s);self.assertIn('source_height',s);self.assertIn('final_png_failed',s);self.assertIn('30000',s);self.assertIn('-final',s)
   self.assertNotIn('getImage(',s);self.assertNotIn('Resize(',s)
 def test_metadata_bundle_is_separate_and_explicit(self):
  s=(ROOT/'_shared/reporting.ps1').read_text();part=s[s.index('function Export-VVMetadataBundle'):s.index('function Export-VVBundle')]
  self.assertIn('review-metadata.zip',part);self.assertIn('bundle-manifest.json',part);self.assertIn('images_included=$false',part);self.assertIn('binary-artifact',part);self.assertIn('Metadata-only review bundle',part)
  allowed=part[part.index(".ToLowerInvariant() -notin"):part.index(".ToLowerInvariant() -notin")+160]
  self.assertNotIn("'.png'",allowed);self.assertNotIn("'.xlsx'",allowed)
  self.assertIn('REVIEW BUNDLE (NO IMAGES)',s)
if __name__=='__main__':unittest.main()
