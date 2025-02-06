```sh
cd frontend
npm i
npm run build
cd ..

sudo ctr run --net-host --rm --mount type=bind,src=$(pwd),dst=/workspace,options=rbind:rw gcr.io/kaniko-project/executor:latest kaniko-executor /kaniko/executor --dockerfile=/workspace/Dockerfile --context=/workspace --no-push --build-arg pkg=docs-app --tarPath=/workspace/doc-manager-v3.tar --destination=giuliohome/doc-manager:v3.0

sudo ctr image import doc-manager-v3.tar
sudo ctr c create --net-host docker.io/giuliohome/doc-manager:v3.0 doc-manager
sudo ctr t start doc-manager
```

<img width="1906" alt="image" src="https://github.com/user-attachments/assets/e8c7cecb-adac-4f5f-9f94-143b0e867e3d" />

<img width="1897" alt="image" src="https://github.com/user-attachments/assets/64cfae0f-47fa-40b5-a743-b0f02e160b78" />
