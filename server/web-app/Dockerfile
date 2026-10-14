# Build context is server/ (the npm workspace root): `docker build -f web-app/Dockerfile .`
FROM node:24-alpine AS builder

WORKDIR /app

# The workspace installs against the one lockfile at its root. backend-client ships TypeScript source and is linked
# as is; e2e only has to exist for `npm ci` to accept the lockfile.
COPY package.json package-lock.json ./
COPY backend-client ./backend-client/
COPY e2e/package.json ./e2e/
COPY web-app/package.json ./web-app/
RUN npm ci --workspace web-app --include-workspace-root

COPY web-app ./web-app/
WORKDIR /app/web-app
RUN npm run build

FROM nginx:1.31.6-alpine AS runner

COPY --from=builder /app/web-app/dist /usr/share/nginx/html
COPY web-app/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 80
