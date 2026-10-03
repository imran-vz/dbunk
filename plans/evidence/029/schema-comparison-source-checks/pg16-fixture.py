import hashlib,json,os,socket,subprocess,time,uuid
from pathlib import Path
BASE=Path('/private/tmp/dbunk-native-comparison-runtime-20261003')
PREFIX=BASE/'installed'
EVIDENCE=Path('/Users/imran/projects/Code/dbunk/plans/evidence/029/schema-comparison-source-checks')
def run(args, **kw):
    return subprocess.check_output([str(arg) for arg in args],text=True,stderr=subprocess.STDOUT,**kw).strip()
with socket.socket() as check:
    check.bind(('127.0.0.1',15434))
instance=str(uuid.uuid4())
root=BASE/('fixture-'+instance)
root.mkdir(mode=0o700)
owned={'version':1,'kind':'dbunk-native-schema-comparison','instance_uuid':instance,'root':str(root),'host':'127.0.0.1','port':15434,'database':'schema_compare_native','user':'dbunk','executable':str(PREFIX/'bin/postgres'),'executable_sha256':hashlib.file_digest((PREFIX/'bin/postgres').open('rb'),'sha256').hexdigest()}
marker=root/'.dbunk-native-comparison'
marker.write_text(json.dumps(owned,indent=2)+'\n')
password=root/'initial-password'
password.write_text('dbunk\n');password.chmod(0o600)
try:
    run([PREFIX/'bin/initdb','-L',PREFIX/'share/postgresql','-D',root/'data','-U','dbunk','--pwfile',password,'--auth-host=scram-sha-256','--auth-local=scram-sha-256','--encoding=UTF8','--locale=C'])
finally:
    password.unlink()
with (root/'data/postgresql.conf').open('a') as config:
    config.write("\nlisten_addresses='127.0.0.1'\nport=15434\nunix_socket_directories=''\nmax_connections=32\n")
with (root/'postgres.log').open('w') as log:
    process=subprocess.Popen([str(PREFIX/'bin/postgres'),'-D',str(root/'data')],stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
owned['pid']=process.pid
marker.write_text(json.dumps(owned,indent=2)+'\n')
(EVIDENCE/'pg16-fixture-owner.json').write_text(json.dumps(owned,indent=2)+'\n')
def check():
    assert json.loads(marker.read_text())==owned
    assert run(['/bin/ps','-p',str(process.pid),'-o','command='])==f"{owned['executable']} -D {root/'data'}"
    assert hashlib.file_digest((PREFIX/'bin/postgres').open('rb'),'sha256').hexdigest()==owned['executable_sha256']
    lines=(root/'data/postmaster.pid').read_text().splitlines()
    assert lines[0]==str(process.pid) and lines[1]==str(root/'data') and lines[3]=='15434' and lines[5]=='127.0.0.1'
def sql(query,database='schema_compare_native'):
    check()
    return run([PREFIX/'bin/psql','-X','-qAt','-v','ON_ERROR_STOP=1','-h','127.0.0.1','-p','15434','-U','dbunk','-d',database],input=query,env=dict(os.environ,PGPASSWORD='dbunk'))
for _ in range(100):
    if process.poll() is not None: raise RuntimeError('Owned PostgreSQL exited')
    try:
        sql('select 1','postgres');break
    except (OSError,AssertionError,subprocess.SubprocessError): time.sleep(.1)
else: raise RuntimeError('Owned fixture did not become ready')
sql('create database schema_compare_native','postgres')
sql("create schema dbunk_native_comparison_fixture; create table dbunk_native_comparison_fixture.identity(instance_uuid text primary key not null); insert into dbunk_native_comparison_fixture.identity values ('"+instance+"');")
print(json.dumps(owned,indent=2))
print(sql('select version(); select instance_uuid from dbunk_native_comparison_fixture.identity;'))
