```sh
cd frontend
npm i
npm run build
cd ..
```

## Using docker engine on Gitpod

```sh
docker build -t giuliohome/doc-manager:v4.1.1 .
export AZURE_STORAGE_ACCOUNT=youraccount
export AZURE_STORAGE_ACCESS_KEY=yourkey
docker run -p 8080:8080 -e AZURE_STORAGE_ACCOUNT=$AZURE_STORAGE_ACCOUNT -e AZURE_STORAGE_ACCESS_KEY=$AZURE_STORAGE_ACCESS_KEY giuliohome/doc-manager:v4.1.1
```

## TL:DR; Using containerd and kaniko

```
sudo mkdir /kcache
sudo ctr i pull gcr.io/kaniko-project/warmer:latest
sudo ctr run --net-host --rm --mount type=bind,src=$(pwd),dst=/workspace,options=rbind:rw --mount type=bind,src=/kcache,dst=/cache,options=rbind:rw gcr.io/kaniko-project/warmer:latest kaniko-warmer /kaniko/warmer --cache-dir=/cache --image=docker.io/rust:1-slim-bookworm --skip-tls-verify-registry index.docker.io --dockerfile=/workspace/Dockerfile

sudo ctr i pull gcr.io/kaniko-project/executor:latest
sudo ctr run --net-host --rm --mount type=bind,src=$(pwd),dst=/workspace,options=rbind:rw --mount type=bind,src=/kcache,dst=/cache,options=rbind:rw gcr.io/kaniko-project/executor:latest kaniko-executor /kaniko/executor -cache-dir=/cache --dockerfile=/workspace/Dockerfile --context=/workspace --no-push --skip-tls-verify --build-arg pkg=docs-app --tarPath=/workspace/doc-manager-v4.1.1.tar --destination=giuliohome/doc-manager:v4.1.1 --cache=true --cache-repo=giuliohome/doc-manager:v4.1.1 --no-push-cache

sudo ctr image import doc-manager-v4.1.1.tar
sudo ctr c create --net-host docker.io/giuliohome/doc-manager:v4.1.1 doc-manager
sudo ctr t start doc-manager
```

<img width="1906" alt="image" src="https://github.com/user-attachments/assets/e8c7cecb-adac-4f5f-9f94-143b0e867e3d" />

<img width="1897" alt="image" src="https://github.com/user-attachments/assets/64cfae0f-47fa-40b5-a743-b0f02e160b78" />
