const fs=require('fs');const {format}=require('/Users/imran/projects/Code/dbunk/node_modules/sql-formatter');
const cases=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
console.log(JSON.stringify(cases.map(c=>{try{return {...c,js:format(c.sql,{language:'postgresql',keywordCase:'upper',tabWidth:2,useTabs:false,linesBetweenQueries:2})}}catch(e){return {...c,jsError:e.message}}}),null,2));